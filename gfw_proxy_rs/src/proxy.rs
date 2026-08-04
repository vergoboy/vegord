use std::cmp::Ordering as CmpOrdering;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::str::FromStr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::time::{timeout, Instant};

use crate::config::Config;
use crate::discord::DiscordManager;
use crate::doh::DohClient;
use crate::fragment::send_fragmented_async;
use crate::stats::{now_iso, StatsManager};

pub struct ProxyServer {
    config: Config,
    doh: Arc<DohClient>,
    discord: Arc<DiscordManager>,
    stats: Arc<StatsManager>,
}

impl ProxyServer {
    pub fn new(
        config: Config,
        doh: Arc<DohClient>,
        discord: Arc<DiscordManager>,
        stats: Arc<StatsManager>,
    ) -> Arc<Self> {
        Arc::new(Self {
            config,
            doh,
            discord,
            stats,
        })
    }

    pub async fn run(self: Arc<Self>, listener: TcpListener) -> std::io::Result<()> {
        loop {
            match listener.accept().await {
                Ok((socket, _peer_addr)) => {
                    let _ = socket.set_nodelay(true);
                    let server = Arc::clone(&self);
                    tokio::spawn(async move {
                        server.handle_client(socket).await;
                    });
                }
                Err(err) => {
                    eprintln!("[{}] [ACCEPT ERR] {}", now_iso(), err);
                }
            }
        }
    }

    async fn handle_client(&self, mut client: TcpStream) {
        let mut buf = [0u8; 16384];
        let read_timeout = Duration::from_secs(self.config.socket_timeout_sec);

        let n = match timeout(read_timeout, client.read(&mut buf)).await {
            Ok(Ok(n)) if n > 0 => n,
            _ => return,
        };

        let data = &buf[..n];

        if data[0] == 5 {
            self.handle_socks5(client, data, n).await;
            return;
        }

        if data.starts_with(b"CONNECT ") {
            self.handle_http_connect(client, data).await;
            return;
        }

        if data.starts_with(b"GET ")
            || data.starts_with(b"POST ")
            || data.starts_with(b"HEAD ")
            || data.starts_with(b"OPTIONS ")
            || data.starts_with(b"PUT ")
            || data.starts_with(b"DELETE ")
            || data.starts_with(b"PATCH ")
            || data.starts_with(b"TRACE ")
        {
            self.handle_http_redirect(client, data).await;
            return;
        }

        println!("[{}] [UNKNOWN] header: {:?}", now_iso(), &data[..std::cmp::min(10, data.len())]);
        let _ = client
            .write_all(b"HTTP/1.1 400 Bad Request\r\nProxy-agent: VegordProxy/3.0\r\n\r\n")
            .await;
    }

    async fn handle_socks5(&self, mut client: TcpStream, data: &[u8], n: usize) {
        if n < 2 || !data[2..2 + data[1] as usize].contains(&0) {
            let _ = client.write_all(b"\x05\xff").await;
            return;
        }
        if client.write_all(b"\x05\x00").await.is_err() {
            return;
        }

        let mut req_buf = [0u8; 16384];
        let read_timeout = Duration::from_secs(self.config.socket_timeout_sec);
        let req_n = match timeout(read_timeout, client.read(&mut req_buf)).await {
            Ok(Ok(n)) if n >= 4 => n,
            _ => return,
        };
        let req = &req_buf[..req_n];

        if req[0] != 5 {
            return;
        }

        let cmd = req[1];
        let atype = req[3];

        let (host, port) = match atype {
            1 => {
                if req_n < 10 {
                    return;
                }
                let ip = Ipv4Addr::new(req[4], req[5], req[6], req[7]);
                (ip.to_string(), u16::from_be_bytes([req[8], req[9]]))
            }
            3 => {
                let dlen = req[4] as usize;
                if req_n < 5 + dlen + 2 {
                    return;
                }
                let h = match std::str::from_utf8(&req[5..5 + dlen]) {
                    Ok(s) => s.to_string(),
                    Err(_) => return,
                };
                let p = u16::from_be_bytes([req[5 + dlen], req[6 + dlen]]);
                (h, p)
            }
            4 => {
                if req_n < 22 {
                    return;
                }
                let mut octets = [0u8; 16];
                octets.copy_from_slice(&req[4..20]);
                let ip = std::net::Ipv6Addr::from(octets);
                (ip.to_string(), u16::from_be_bytes([req[20], req[21]]))
            }
            _ => {
                let _ = client
                    .write_all(b"\x05\x08\x00\x01\x00\x00\x00\x00\x00\x00")
                    .await;
                return;
            }
        };

        match cmd {
            1 => {
                // SOCKS5 CONNECT
                println!("[{}] [SOCKS5] {}:{}", now_iso(), host, port);
                self.connect_and_relay(client, &host, port).await;
            }
            3 => {
                // SOCKS5 UDP ASSOCIATE (For Discord WebRTC Audio/Video!)
                println!("[{}] [SOCKS5 UDP] client requested UDP associate for {}:{}", now_iso(), host, port);
                self.handle_udp_associate(client).await;
            }
            _ => {
                let _ = client
                    .write_all(b"\x05\x07\x00\x01\x00\x00\x00\x00\x00\x00")
                    .await;
            }
        }
    }

    async fn handle_udp_associate(&self, mut client: TcpStream) {
        let udp_socket = match UdpSocket::bind("127.0.0.1:0").await {
            Ok(s) => s,
            Err(_) => {
                let _ = client
                    .write_all(b"\x05\x01\x00\x01\x00\x00\x00\x00\x00\x00")
                    .await;
                return;
            }
        };

        let local_addr = match udp_socket.local_addr() {
            Ok(a) => a,
            Err(_) => {
                let _ = client
                    .write_all(b"\x05\x01\x00\x01\x00\x00\x00\x00\x00\x00")
                    .await;
                return;
            }
        };

        let mut resp = vec![0x05, 0x00, 0x00, 0x01];
        match local_addr.ip() {
            IpAddr::V4(v4) => resp.extend_from_slice(&v4.octets()),
            _ => resp.extend_from_slice(&[0, 0, 0, 0]),
        }
        resp.extend_from_slice(&local_addr.port().to_be_bytes());

        if client.write_all(&resp).await.is_err() {
            return;
        }

        // Spawn UDP relay loop while TCP connection remains active. The relay
        // is voice-health-aware (spec section 5.2): a heartbeat task watches
        // for traffic from the target and, when the route is dead, transparently
        // fails over to the next-best Discord IP from the ranked pool instead of
        // silently breaking the voice channel.
        let stats = Arc::clone(&self.stats);
        let discord = Arc::clone(&self.discord);
        let udp_heartbeat_sec = self.config.udp_heartbeat_sec;
        let udp_loss_window_sec = self.config.udp_loss_window_sec;
        tokio::spawn(async move {
            let mut buf = [0u8; 65535];
            let mut client_udp_addr: Option<SocketAddr> = None;
            // target we relay to (Discord voice server), mutable for failover.
            let mut target_addr: Option<SocketAddr> = None;
            let mut last_target_seen: Option<Instant> = None;
            let mut consecutive_misses: u32 = 0;
            let mut last_traffic: Option<Instant> = None;
            // Loss-window bookkeeping: count 1-second ticks without a target
            // packet and feed the resulting ratio into the DiscordManager so the
            // voice path is scored by loss, not just RTT (spec section 5.2).
            let mut tick_count: u64 = 0;
            let mut miss_count: u64 = 0;
            let mut got_target_packet: bool = false;

            loop {
                let recv = tokio::time::timeout(
                    Duration::from_secs(1),
                    udp_socket.recv_from(&mut buf),
                )
                .await;

                match recv {
                    Ok(Ok((n, src_addr))) => {
                        if client_udp_addr.is_none() {
                            client_udp_addr = Some(src_addr);
                        }

                        if Some(src_addr) == client_udp_addr {
                            // Packet coming from client -> extract SOCKS5 UDP header
                            // and forward to the (possibly failover-swapped) target.
                            if n > 10 {
                                let frag = buf[2];
                                if frag == 0 {
                                    let atype = buf[3];
                                    let mut header_len = 0;
                                    let mut target_ip_opt: Option<IpAddr> = None;
                                    let mut target_port: u16 = 0;

                                    if atype == 1 && n >= 10 {
                                        let ip = Ipv4Addr::new(buf[4], buf[5], buf[6], buf[7]);
                                        target_ip_opt = Some(IpAddr::V4(ip));
                                        target_port = u16::from_be_bytes([buf[8], buf[9]]);
                                        header_len = 10;
                                    } else if atype == 3 {
                                        let dlen = buf[4] as usize;
                                        if n >= 5 + dlen + 2 {
                                            if let Ok(domain) = std::str::from_utf8(&buf[5..5 + dlen]) {
                                                if let Ok(ip) = IpAddr::from_str(domain) {
                                                    target_ip_opt = Some(ip);
                                                }
                                            }
                                            target_port = u16::from_be_bytes([buf[5 + dlen], buf[6 + dlen]]);
                                            header_len = 5 + dlen + 2;
                                        }
                                    }

                                    if let (Some(target_ip), true) = (target_ip_opt, header_len > 0 && n > header_len) {
                                        let payload = &buf[header_len..n];
                                        // Discord sends to the literal voice IP; remember the
                                        // original so failover can swap the IP silently.
                                        if target_addr.is_none() {
                                            target_addr = Some(SocketAddr::new(target_ip, target_port));
                                        }
                                        let dest_addr = target_addr
                                            .map(|mut a| {
                                                a.set_ip(target_ip);
                                                a
                                            })
                                            .unwrap_or(SocketAddr::new(target_ip, target_port));

                                        let _ = udp_socket.send_to(payload, dest_addr).await;
                                        stats.record_ul(target_ip, payload.len() as u64);
                                        last_traffic = Some(Instant::now());
                                    }
                                }
                            }
                        } else {
                            // Packet coming from target server -> encapsulate in
                            // SOCKS5 UDP header & send to client.
                            last_target_seen = Some(Instant::now());
                            consecutive_misses = 0;
                            got_target_packet = true;
                            if let Some(c_addr) = client_udp_addr {
                                let mut packet = vec![0x00, 0x00, 0x00, 0x01];
                                match src_addr.ip() {
                                    IpAddr::V4(v4) => packet.extend_from_slice(&v4.octets()),
                                    _ => packet.extend_from_slice(&[0, 0, 0, 0]),
                                }
                                packet.extend_from_slice(&src_addr.port().to_be_bytes());
                                packet.extend_from_slice(&buf[..n]);

                                let _ = udp_socket.send_to(&packet, c_addr).await;
                                stats.record_dl(src_addr.ip(), n as u64);
                            }
                        }
                    }
                    Ok(Err(_)) => {
                        // Transient UDP error is not fatal: keep the channel alive.
                        continue;
                    }
                    Err(_) => {
                        // recv timeout -> run heartbeat bookkeeping.
                        if last_traffic.is_none() {
                            continue;
                        }
                        // Loss-window measurement: one 1s tick happened and no
                        // target packet arrived in it (unless got_target_packet).
                        tick_count += 1;
                        if !got_target_packet {
                            miss_count += 1;
                        }
                        got_target_packet = false;
                        if tick_count >= udp_heartbeat_sec {
                            let pct = (miss_count as f64 / tick_count as f64) * 100.0;
                            if let Some(t) = target_addr {
                                discord.note_loss_sample(t.ip(), pct);
                                if pct > 0.0 {
                                    println!(
                                        "[{}] [VOICE] loss for {} = {:.1}% (window={}s)",
                                        now_iso(),
                                        t.ip(),
                                        pct,
                                        tick_count
                                    );
                                }
                            }
                            tick_count = 0;
                            miss_count = 0;
                        }
                        let now = Instant::now();
                        let idle = now.duration_since(last_traffic.unwrap());
                        if idle > Duration::from_secs(udp_loss_window_sec) {
                            let since_target = last_target_seen
                                .map(|t| now.duration_since(t))
                                .unwrap_or(Duration::from_secs(0));
                            if since_target > Duration::from_secs(udp_loss_window_sec) {
                                consecutive_misses += 1;
                                if consecutive_misses >= 2 {
                                    // Route is dead: fail over to the next-best
                                    // Discord IP from the ranked pool.
                                    if let Some(t) = target_addr {
                                        let next = discord.next_best_ip(Some(t.ip()));
                                        if let Some(next_ip) = next {
                                            if next_ip != t.ip() {
                                                target_addr = Some(SocketAddr::new(next_ip, t.port()));
                                                consecutive_misses = 0;
                                                println!(
                                                    "[{}] [VOICE FAILOVER] {} -> {} (heartbeat missed)",
                                                    now_iso(),
                                                    t.ip(),
                                                    next_ip
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        // Heartbeat: log voice health periodically.
                        if let Some(t) = target_addr {
                            if last_target_seen.is_none() {
                                println!(
                                    "[{}] [VOICE] relay active to {}, waiting for first target packet",
                                    now_iso(),
                                    t
                                );
                            }
                        }
                    }
                }
            }
        });

        // Keep TCP connection open to hold UDP association alive
        let mut dummy = [0u8; 1];
        let _ = client.read(&mut dummy).await;
    }

    async fn handle_http_connect(&self, mut client: TcpStream, data: &[u8]) {
        let line = match std::str::from_utf8(data) {
            Ok(s) => s.lines().next().unwrap_or(""),
            Err(_) => return,
        };

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 2 {
            return;
        }

        let target = parts[1];
        let (host, port_str) = match target.split_once(':') {
            Some((h, p)) => (h, p),
            None => (target, "443"),
        };

        let port: u16 = port_str.parse().unwrap_or(443);
        println!("[{}] [CONNECT] {}:{}", now_iso(), host, port);

        let is_gateway = DohClient::is_gateway_host(host);
        if is_gateway {
            self.doh.note_gateway_connect();
        }

        let targets = self.resolve_targets(host).await;
        let mut backend: Option<TcpStream> = None;
        if !targets.is_empty() {
            let max_attempts = self.config.connect_retries.max(1);
            for attempt in 0..max_attempts {
                match self.connect_target(targets[0], port).await {
                    Ok(stream) => {
                        backend = Some(stream);
                        break;
                    }
                    Err(_) if attempt + 1 < max_attempts => {
                        tokio::time::sleep(Duration::from_millis(80)).await;
                    }
                    Err(_) => break,
                }
            }
        }

        if let Some(backend) = backend {
            let resp = b"HTTP/1.1 200 Connection established\r\nProxy-agent: VegordProxy/3.0\r\n\r\n";
            if client.write_all(resp).await.is_ok() {
                self.relay_bidirectional(client, backend, host, port, &targets)
                    .await;
            }
            if is_gateway {
                self.doh.note_gateway_disconnect();
            }
            return;
        }

        if is_gateway {
            self.doh.note_gateway_disconnect();
        }

        self.stats.conn_filtered.fetch_add(1, Ordering::Relaxed);
        let _ = client
            .write_all(b"HTTP/1.1 502 Bad Gateway\r\nProxy-agent: VegordProxy/3.0\r\n\r\n")
            .await;
    }

    async fn handle_http_redirect(&self, mut client: TcpStream, data: &[u8]) {
        let line = match std::str::from_utf8(data) {
            Ok(s) => s.lines().next().unwrap_or(""),
            Err(_) => return,
        };

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            let method = parts[0];
            let url = parts[1].replace("http://", "https://");
            println!("[{}] [REDIRECT] {} http -> https {}", now_iso(), method, url);
            let resp = format!(
                "HTTP/1.1 302 Found\r\nLocation: {}\r\nProxy-agent: VegordProxy/3.0\r\n\r\n",
                url
            );
            let _ = client.write_all(resp.as_bytes()).await;
        }
    }

    async fn resolve_targets(&self, host: &str) -> Vec<IpAddr> {
        if let Ok(ip) = IpAddr::from_str(host) {
            return vec![ip];
        }

        if DiscordManager::is_discord_domain(host) {
            let mut ips = Vec::new();
            if let Some(best_ip) = self.discord.get_best_ip() {
                ips.push(best_ip);
            }
            let mut rest: Vec<IpAddr> = {
                let mut snap = self.discord.get_ips_snapshot();
                snap.sort_by(|a, b| match (a.1, b.1) {
                    (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(CmpOrdering::Equal),
                    (Some(_), None) => CmpOrdering::Less,
                    (None, Some(_)) => CmpOrdering::Greater,
                    (None, None) => CmpOrdering::Equal,
                });
                snap.into_iter().map(|(ip, _, _)| ip).collect()
            };
            for ip in rest.drain(..) {
                if !ips.contains(&ip) {
                    ips.push(ip);
                }
            }
            if !ips.is_empty() {
                println!("[{}] [DISCORD] routing {} via {:?}", now_iso(), host, ips);
                return ips;
            }
        }

        self.doh.query_all(host).await.unwrap_or_default()
    }

    async fn connect_target(&self, ip: IpAddr, port: u16) -> std::io::Result<TcpStream> {
        self.stats.conn_total.fetch_add(1, Ordering::Relaxed);
        let addr = SocketAddr::new(ip, port);

        // Phase-specific connect deadline (spec section 5.3): the TCP connect
        // has its own budget independent of the handshake / idle / bulk phases.
        let timeout_sec = self.config.connect_deadline_sec;

        let stream_res = timeout(Duration::from_secs(timeout_sec), TcpStream::connect(addr)).await;
        match stream_res {
            Ok(Ok(stream)) => {
                let _ = stream.set_nodelay(true);
                self.stats.conn_success.fetch_add(1, Ordering::Relaxed);
                Ok(stream)
            }
            Ok(Err(e)) => {
                self.stats.conn_filtered.fetch_add(1, Ordering::Relaxed);
                println!("[{}] [FILTERED] ({}):{} - {}", now_iso(), ip, port, e);
                Err(e)
            }
            Err(_) => {
                self.stats.conn_filtered.fetch_add(1, Ordering::Relaxed);
                println!("[{}] [FILTERED TIMEOUT] ({}):{}", now_iso(), ip, port);
                Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "Connection timed out",
                ))
            }
        }
    }

    async fn connect_and_relay(&self, mut client: TcpStream, host: &str, port: u16) {
        let is_gateway = DohClient::is_gateway_host(host);
        if is_gateway {
            self.doh.note_gateway_connect();
        }

        let targets = self.resolve_targets(host).await;
        if targets.is_empty() {
            if is_gateway {
                self.doh.note_gateway_disconnect();
            }
            let _ = client
                .write_all(b"\x05\x04\x00\x01\x00\x00\x00\x00\x00\x00")
                .await;
            return;
        }

        // The GFW intermittently resets individual TCP connections, especially
        // to Cloudflare-fronted domains (e.g. api.vencord.dev). A single reset
        // would surface as a failed request, so retry the connect a few times
        // to absorb the sporadic RSTs instead of giving up immediately.
        let mut backend: Option<TcpStream> = None;
        let max_attempts = self.config.connect_retries.max(1);
        for attempt in 0..max_attempts {
            match self.connect_target(targets[0], port).await {
                Ok(stream) => {
                    backend = Some(stream);
                    break;
                }
                Err(_) if attempt + 1 < max_attempts => {
                    println!(
                        "[{}] [RETRY] {} ({}) connect attempt {}/{}",
                        now_iso(),
                        host,
                        targets[0],
                        attempt + 1,
                        max_attempts
                    );
                    tokio::time::sleep(Duration::from_millis(80)).await;
                }
                Err(_) => break,
            }
        }

        if let Some(backend) = backend {
            let resp = b"\x05\x00\x00\x01\x00\x00\x00\x00\x00\x00";
            if client.write_all(resp).await.is_ok() {
                self.relay_bidirectional(client, backend, host, port, &targets)
                    .await;
            }
            if is_gateway {
                self.doh.note_gateway_disconnect();
            }
            return;
        }

        if is_gateway {
            self.doh.note_gateway_disconnect();
        }

        let _ = client
            .write_all(b"\x05\x04\x00\x01\x00\x00\x00\x00\x00\x00")
            .await;
    }

    async fn relay_bidirectional(
        &self,
        mut client: TcpStream,
        mut backend: TcpStream,
        host: &str,
        port: u16,
        targets: &[IpAddr],
    ) {
        // Read first payload from client (TLS Client Hello)
        let mut first_buf = [0u8; 16384];
        let read_timeout = Duration::from_secs(self.config.socket_timeout_sec);

        let first_n = match timeout(read_timeout, client.read(&mut first_buf)).await {
            Ok(Ok(n)) if n > 0 => n,
            _ => return,
        };

        // Send the ClientHello with TLS SNI fragmentation. If the connection is
        // reset mid-handshake (GFW RST against Cloudflare etc.), the client has
        // not received anything yet — it is still waiting for the ServerHello —
        // so we can transparently reconnect to the same or an alternate IP and
        // resend the exact same ClientHello.
        let mut ip_idx = 0usize;
        let max_relay_attempts = self.config.relay_retries.max(1);
        let mut delivered = false;
        for attempt in 0..max_relay_attempts {
            if attempt > 0 {
                // GFW RSTs tend to land in short bursts that hit every attempt
                // within the same window, so space retries out a bit.
                tokio::time::sleep(Duration::from_millis(self.config.relay_retry_sleep_ms)).await;
                if targets.len() > 1 {
                    ip_idx = (ip_idx + 1) % targets.len();
                }
                let ip = targets[ip_idx];
                println!("[{}] [FRAG RETRY] {} via {}", now_iso(), host, ip);
                match self.connect_target(ip, port).await {
                    Ok(stream) => backend = stream,
                    Err(_) => continue,
                }
            }

            match send_fragmented_async(
                &mut backend,
                &first_buf[..first_n],
                self.config.num_fragment,
                self.config.fragment_sleep_ms,
            )
            .await
            {
                Ok(()) => {}
                Err(err) => {
                    eprintln!(
                        "[{}] [FRAG ERR] {} (attempt {}/{})",
                        now_iso(),
                        err,
                        attempt + 1,
                        max_relay_attempts
                    );
                    if attempt + 1 >= max_relay_attempts {
                        break;
                    }
                    continue;
                }
            }

            // The ClientHello write may succeed into the kernel buffer even when
            // the GFW RST is already in flight; the reset only surfaces as an
            // error/EOF on the next read (while waiting for the ServerHello).
            // Conversely, DPI may silently drop the ServerHello packets, leaving
            // the backend connection half-open (no error, no data). Until any
            // backend byte reaches the client, the client is still waiting for
            // the ServerHello, so we can transparently replay the exact same
            // ClientHello on the next IP.
            let is_tls = first_buf[0] == 0x16 && first_n >= 6 && first_buf[5] == 0x01;
            if is_tls {
                let handshake_timeout =
                    Duration::from_secs(self.config.relay_handshake_timeout_sec);
                let mut hello = [0u8; 8192];
                match timeout(handshake_timeout, backend.read(&mut hello)).await {
                    Ok(Ok(0)) | Ok(Err(_)) => {
                        eprintln!(
                            "[{}] [FRAG HANDSHAKE RST] {} (attempt {}/{})",
                            now_iso(),
                            host,
                            attempt + 1,
                            max_relay_attempts
                        );
                        if attempt + 1 >= max_relay_attempts {
                            break;
                        }
                        continue;
                    }
                    Ok(Ok(n)) => {
                        if client.write_all(&hello[..n]).await.is_err() {
                            break;
                        }
                        self.stats.record_dl(targets[ip_idx], n as u64);
                    }
                    Err(_) => {
                        // ServerHello never arrived: either our fragmented
                        // ClientHello was dropped or DPI throttled the response
                        // during a probe burst. Either way the handshake is
                        // dead; do not mark it delivered.
                        eprintln!(
                            "[{}] [FRAG HANDSHAKE TIMEOUT] {} (attempt {}/{})",
                            now_iso(),
                            host,
                            attempt + 1,
                            max_relay_attempts
                        );
                        if attempt + 1 >= max_relay_attempts {
                            break;
                        }
                        continue;
                    }
                }
            }

            delivered = true;
            break;
        }
        if !delivered {
            return;
        }

        let ip = targets[ip_idx];
        self.stats.record_ul(ip, first_n as u64);

        // Bi-directional async forwarding, half-close aware (spec section 5.1).
        //
        // The old code used `tokio::select!`, which cancelled the other task the
        // moment either direction finished — dropping buffered upload bytes when
        // a server sent an early response / 100-continue. Instead each direction
        // runs to completion independently (tokio::join!). When one direction
        // hits EOF it performs a real TCP half-close (shutdown) on the opposite
        // write half, and the connection is only torn down once BOTH directions
        // have finished. An overall bulk-transfer deadline is the only way the
        // relay is force-closed (guards genuinely dead connections).
        let (mut cr, mut cw) = client.split();
        let (mut br, mut bw) = backend.split();

        let stats_ul = Arc::clone(&self.stats);
        let stats_dl = Arc::clone(&self.stats);

        let client_to_backend = async move {
            let mut buf = [0u8; 16384];
            loop {
                match cr.read(&mut buf).await {
                    Ok(0) => {
                        let _ = bw.shutdown().await;
                        break;
                    }
                    Ok(n) => {
                        if bw.write_all(&buf[..n]).await.is_err() {
                            break;
                        }
                        stats_ul.record_ul(ip, n as u64);
                    }
                    Err(_) => break,
                }
            }
        };

        let backend_to_client = async move {
            let mut buf = [0u8; 16384];
            loop {
                match br.read(&mut buf).await {
                    Ok(0) => {
                        let _ = cw.shutdown().await;
                        break;
                    }
                    Ok(n) => {
                        if cw.write_all(&buf[..n]).await.is_err() {
                            break;
                        }
                        stats_dl.record_dl(ip, n as u64);
                    }
                    Err(_) => break,
                }
            }
        };

        let deadline = tokio::time::sleep(Duration::from_secs(self.config.bulk_transfer_deadline_sec));
        tokio::pin!(deadline);
        let relay = async {
            let _ = tokio::join!(client_to_backend, backend_to_client);
        };
        tokio::pin!(relay);
        tokio::select! {
            _ = &mut deadline => {
                println!(
                    "[{}] [RELAY DEADLINE] {}:{} exceeded {}s, closing",
                    now_iso(),
                    host,
                    port,
                    self.config.bulk_transfer_deadline_sec
                );
            }
            _ = &mut relay => {}
        }
    }
}
