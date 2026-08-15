// Local TLS MITM for Discord hosts.
//
// The ISP's DPI fingerprints the app's BoringSSL ClientHello and resets the
// connection even when it is fragmented (OpenSSL/rustls ClientHellos pass).
// There is no way to change what bytes the app's BoringSSL emits without
// breaking the TLS transcript, so instead of forwarding the app's handshake we
// terminate it locally: the proxy answers with a self-signed certificate for
// the target host (the app must be launched with --ignore-certificate-errors,
// so BoringSSL accepts it over loopback where no DPI can interfere), decrypts
// the app's HTTP/WebSocket stream, and re-encrypts it into a fresh connection
// to the real Discord edge using rustls — a stack whose fragmented ClientHello
// the DPI lets through. The plaintext HTTP/WS bytes are relayed verbatim, so
// no HTTP parsing is needed and keep-alive / h2 / WebSocket all work.
//
// Each connection runs on a dedicated std-IO thread: the handshake writes need
// byte-level control (fragmenting the upstream rustls ClientHello) that does
// not fit the tokio poll model cleanly.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::TcpStream as StdTcpStream;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rcgen::generate_simple_self_signed;
use rustls::pki_types::{
    CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName,
};
use rustls::{
    ClientConfig, ClientConnection, ConnectionCommon, RootCertStore, ServerConfig,
    ServerConnection, SideData,
};

use crate::fragment::send_fragmented_blocking;
use crate::stats::now_iso;

const ALPN: &[&[u8]] = &[b"h2", b"http/1.1"];

/// Generates and caches the self-signed certs and holds the upstream rustls
/// client configuration (public roots + h2/http1 ALPN).
pub struct MitmManager {
    certs: Mutex<HashMap<String, (CertificateDer<'static>, Vec<u8>)>>,
    client_config: Arc<ClientConfig>,
}

impl MitmManager {
    pub fn new() -> Arc<Self> {
        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let mut client_config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        client_config.alpn_protocols = ALPN.iter().map(|p| p.to_vec()).collect();
        Arc::new(Self {
            certs: Mutex::new(HashMap::new()),
            client_config: Arc::new(client_config),
        })
    }

    fn server_config(&self, host: &str) -> io::Result<Arc<ServerConfig>> {
        let (cert, key) = {
            let cached = self.certs.lock().unwrap().get(host).cloned();
            match cached {
                Some(c) => c,
                None => {
                    let key = generate_simple_self_signed(vec![host.to_string()]).map_err(|e| {
                        io::Error::new(io::ErrorKind::Other, format!("cert generation: {e}"))
                    })?;
                    let der = key.cert.der().clone();
                    let pkey = key.key_pair.serialize_der();
                    self.certs
                        .lock()
                        .unwrap()
                        .insert(host.to_string(), (der.clone(), pkey.clone()));
                    (der, pkey)
                }
            }
        };
        let key = PrivateKeyDer::from(PrivatePkcs8KeyDer::from(key));
        let mut cfg = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert], PrivateKeyDer::from(key))
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("server config: {e}")))?;
        cfg.alpn_protocols = ALPN.iter().map(|p| p.to_vec()).collect();
        Ok(Arc::new(cfg))
    }
}

enum ReadOutcome {
    Eof,
    Data,
    Closed, // TLS close_notify received
    Nothing,
}

fn read_and_process<D: SideData>(
    conn: &mut ConnectionCommon<D>,
    stream: &mut StdTcpStream,
) -> io::Result<ReadOutcome> {
    if !conn.wants_read() {
        return Ok(ReadOutcome::Nothing);
    }
    let mut buf = [0u8; 16384];
    match stream.read(&mut buf) {
        Ok(0) => Ok(ReadOutcome::Eof),
        Ok(n) => {
            conn.read_tls(&mut &buf[..n])?;
            let state = conn
                .process_new_packets()
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("tls: {e}")))?;
            if state.peer_has_closed() {
                Ok(ReadOutcome::Closed)
            } else {
                Ok(ReadOutcome::Data)
            }
        }
        Err(e) if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut => {
            Ok(ReadOutcome::Nothing)
        }
        Err(e) => Err(e),
    }
}

fn flush_writes<D: SideData>(conn: &mut ConnectionCommon<D>, stream: &mut StdTcpStream) -> io::Result<()> {
    while conn.wants_write() {
        let mut out = Vec::new();
        conn.write_tls(&mut out)?;
        if !out.is_empty() {
            stream.write_all(&out)?;
        }
    }
    Ok(())
}

/// Drive a TLS handshake to completion. The very first outbound record (the
/// ClientHello for the client side) can be fragmented like a normal relayed
/// ClientHello so the DPI never sees a recognizable unfragmented handshake.
fn drive_handshake<D: SideData>(
    conn: &mut ConnectionCommon<D>,
    stream: &mut StdTcpStream,
    fragment_first_write: Option<(usize, u64)>,
    deadline: Instant,
) -> io::Result<()> {
    let mut first_write = true;
    while conn.is_handshaking() {
        if Instant::now() > deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "TLS handshake deadline exceeded",
            ));
        }
        if conn.wants_read() {
            let mut buf = [0u8; 16384];
            match stream.read(&mut buf) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "peer closed during TLS handshake",
                    ))
                }
                Ok(n) => {
                    conn.read_tls(&mut &buf[..n])?;
                    conn.process_new_packets().map_err(|e| {
                        io::Error::new(io::ErrorKind::InvalidData, format!("tls: {e}"))
                    })?;
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) if e.kind() == io::ErrorKind::TimedOut => {}
                Err(e) => return Err(e),
            }
        }
        if conn.wants_write() {
            let mut out = Vec::new();
            conn.write_tls(&mut out)?;
            if out.is_empty() {
                continue;
            }
            if first_write {
                first_write = false;
                if let Some((num, sleep)) = fragment_first_write {
                    send_fragmented_blocking(stream, &out, num, sleep)?;
                } else {
                    stream.write_all(&out)?;
                    stream.flush()?;
                }
            } else {
                stream.write_all(&out)?;
                stream.flush()?;
            }
        }
        if !conn.wants_read() && !conn.wants_write() && conn.is_handshaking() {
            // Waiting for peer bytes: block briefly so we don't busy-spin.
            let mut buf = [0u8; 16384];
            match stream.read(&mut buf) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "peer closed during TLS handshake",
                    ))
                }
                Ok(n) => {
                    conn.read_tls(&mut &buf[..n])?;
                    conn.process_new_packets().map_err(|e| {
                        io::Error::new(io::ErrorKind::InvalidData, format!("tls: {e}"))
                    })?;
                }
                Err(e)
                    if e.kind() == io::ErrorKind::WouldBlock
                        || e.kind() == io::ErrorKind::TimedOut => {}
                Err(e) => return Err(e),
            }
        }
    }
    flush_writes(conn, stream)
}

fn drain_plaintext<D1: SideData, D2: SideData>(
    src: &mut ConnectionCommon<D1>,
    dst: &mut ConnectionCommon<D2>,
    _src_stream: &mut StdTcpStream,
    dst_stream: &mut StdTcpStream,
    buf: &mut [u8],
) -> io::Result<bool> {
    let mut moved = false;
    loop {
        match src.reader().read(buf) {
            Ok(0) => break,
            Ok(n) => {
                dst.writer().write_all(&buf[..n])?;
                moved = true;
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
            Err(e) => return Err(e),
        }
    }
    if moved {
        flush_writes(dst, dst_stream)?;
    }
    Ok(moved)
}

fn relay_plaintext<D1: SideData, D2: SideData>(
    host: &str,
    server_conn: &mut ConnectionCommon<D1>,
    server_stream: &mut StdTcpStream,
    client_conn: &mut ConnectionCommon<D2>,
    client_stream: &mut StdTcpStream,
    deadline: Instant,
    idle_limit: Duration,
) -> io::Result<()> {
    server_stream.set_read_timeout(Some(Duration::from_millis(150)))?;
    client_stream.set_read_timeout(Some(Duration::from_millis(150)))?;
    let mut buf = [0u8; 16384];

    let mut server_eof = false; // app side
    let mut client_eof = false; // upstream side
    let mut server_close_sent = false;
    let mut client_close_sent = false;
    let mut last_progress = Instant::now();

    loop {
        if Instant::now() > deadline {
            return Ok(());
        }
        // Idle timeout: fires only when the WHOLE relay has been silent for
        // idle_limit (zero bytes both ways). A slow upload keeps resetting it;
        // a connection whose peer silently vanished is reaped and logged.
        if Instant::now().duration_since(last_progress) > idle_limit {
            println!(
                "[{}] [MITM IDLE TIMEOUT] {} no data in either direction for {}s, closing",
                now_iso(),
                host,
                idle_limit.as_secs()
            );
            return Ok(());
        }
        let mut progress = false;

        // app -> upstream
        if !server_eof {
            match read_and_process(server_conn, server_stream)? {
                ReadOutcome::Eof | ReadOutcome::Closed => {
                    server_eof = true;
                    eprintln!("[MITM DBG] app side EOF");
                    progress = true;
                }
                ReadOutcome::Data => progress = true,
                ReadOutcome::Nothing => {}
            }
            if drain_plaintext(server_conn, client_conn, server_stream, client_stream, &mut buf)? {
                progress = true;
            }
        }

        // upstream -> app
        if !client_eof {
            match read_and_process(client_conn, client_stream)? {
                ReadOutcome::Eof | ReadOutcome::Closed => {
                    client_eof = true;
                    eprintln!("[MITM DBG] upstream side EOF");
                    progress = true;
                }
                ReadOutcome::Data => progress = true,
                ReadOutcome::Nothing => {}
            }
            if drain_plaintext(client_conn, server_conn, client_stream, server_stream, &mut buf)? {
                progress = true;
            }
        }

        // Half-close: when one side is done, signal the other end.
        if server_eof && !client_close_sent {
            client_close_sent = true;
            let _ = client_conn.send_close_notify();
            let _ = flush_writes(client_conn, client_stream);
            let _ = client_stream.shutdown(std::net::Shutdown::Write);
        }
        if client_eof && !server_close_sent {
            server_close_sent = true;
            let _ = server_conn.send_close_notify();
            let _ = flush_writes(server_conn, server_stream);
            let _ = server_stream.shutdown(std::net::Shutdown::Write);
        }

        if server_eof && client_eof {
            break;
        }
        if progress {
            last_progress = Instant::now();
        } else {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    Ok(())
}

/// Blocking MITM handler run on a dedicated thread.
#[allow(clippy::too_many_arguments)]
fn run_blocking(
    client: tokio::net::TcpStream,
    backend: tokio::net::TcpStream,
    host: String,
    frag: (usize, u64),
    mitm: Arc<MitmManager>,
    handshake_deadline_sec: u64,
    relay_deadline_sec: u64,
    relay_idle_timeout_sec: u64,
) -> io::Result<()> {
    let mut server_stream: StdTcpStream = client.into_std()?;
    let mut client_stream: StdTcpStream = backend.into_std()?;
    server_stream.set_nonblocking(false)?;
    client_stream.set_nonblocking(false)?;
    server_stream.set_nodelay(true).ok();
    client_stream.set_nodelay(true).ok();

    let start = Instant::now();
    let handshake_deadline = start + Duration::from_secs(handshake_deadline_sec.max(1));

    // 1. Accept the app's BoringSSL handshake with a self-signed cert.
    let server_cfg = mitm.server_config(&host)?;
    let mut server_conn = ServerConnection::new(server_cfg)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("server conn: {e}")))?;
    drive_handshake(&mut server_conn, &mut server_stream, None, handshake_deadline)?;

    // 2. Connect to the real Discord edge with rustls, fragmenting our
    //    ClientHello exactly like a relayed one. The upstream ALPN must mirror
    //    what the app side negotiated, or the HTTP framing bytes won't line up.
    let server_name = ServerName::try_from(host.clone())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, format!("sni: {e}")))?;
    let mut client_config = (*mitm.client_config).clone();
    client_config.alpn_protocols = match server_conn.alpn_protocol() {
        Some(p) => vec![p.to_vec()],
        None => ALPN.iter().map(|p| p.to_vec()).collect(),
    };
    let mut client_conn = ClientConnection::new(Arc::new(client_config), server_name)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("client conn: {e}")))?;
    drive_handshake(
        &mut client_conn,
        &mut client_stream,
        Some(frag),
        handshake_deadline,
    )?;

    println!("[{}] [MITM] {} TLS up (app <=> rustls server <=> rustls client <=> edge)", now_iso(), host);

    // 3. Relay decrypted bytes both ways.
    let relay_deadline = Instant::now() + Duration::from_secs(relay_deadline_sec.max(10));
    let idle_limit = Duration::from_secs(relay_idle_timeout_sec.max(1));
    match relay_plaintext(
        &host,
        &mut *server_conn,
        &mut server_stream,
        &mut *client_conn,
        &mut client_stream,
        relay_deadline,
        idle_limit,
    ) {
        Ok(()) => println!("[{}] [MITM] {} relay ended clean", now_iso(), host),
        Err(e) => println!("[{}] [MITM] {} relay ended: {}", now_iso(), host, e),
    }
    Ok(())
}

/// Spawn the MITM relay for a connected pair of streams on a dedicated thread.
#[allow(clippy::too_many_arguments)]
pub fn spawn(
    client: tokio::net::TcpStream,
    backend: tokio::net::TcpStream,
    host: String,
    frag: (usize, u64),
    mitm: Arc<MitmManager>,
    handshake_deadline_sec: u64,
    relay_deadline_sec: u64,
    relay_idle_timeout_sec: u64,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let host_err = host.clone();
        if let Err(e) = run_blocking(
            client,
            backend,
            host,
            frag,
            mitm,
            handshake_deadline_sec,
            relay_deadline_sec,
            relay_idle_timeout_sec,
        ) {
            eprintln!(
                "[{}] [MITM ERR] {} - {}",
                now_iso(),
                host_err,
                e.to_string().lines().next().unwrap_or("unknown")
            );
        }
    })
}
