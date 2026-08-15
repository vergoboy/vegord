use std::cmp::Ordering as CmpOrdering;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::str::FromStr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{timeout, Instant};

use crate::config::{Config, RelayConfig};
use crate::discord::DiscordManager;
use crate::doh::DohClient;
use crate::fragment::send_fragmented_async;
use crate::mitm::{self, MitmManager};
use crate::stats::{now_iso, StatsManager};
use crate::tun::{self, TunManager};

pub struct ProxyServer {
    config: Config,
    doh: Arc<DohClient>,
    discord: Arc<DiscordManager>,
    stats: Arc<StatsManager>,
    tun: Arc<TunManager>,
    mitm: Arc<MitmManager>,
    // Temporary fragmentation override used by the preset benchmark phase to
    // try several (num_fragment, sleep_ms) configs on real traffic without
    // touching the process-wide config. None = use config defaults.
    frag_override: Arc<parking_lot::RwLock<Option<(usize, u64)>>>,
}

impl ProxyServer {
    pub fn new(
        config: Config,
        doh: Arc<DohClient>,
        discord: Arc<DiscordManager>,
        stats: Arc<StatsManager>,
        frag_override: Arc<parking_lot::RwLock<Option<(usize, u64)>>>,
        tun: Arc<TunManager>,
        mitm: Arc<MitmManager>,
    ) -> Arc<Self> {
        Arc::new(Self {
            config,
            doh,
            discord,
            stats,
            tun,
            mitm,
            frag_override,
        })
    }

    /// SO_MARK for outbound sockets when the split tunnel is active, so the
    /// proxy's own connections bypass the Discord TUN routes (loop avoidance).
    fn fwmark(&self) -> Option<u32> {
        if self.config.tun_split_enabled {
            Some(self.config.tun_fwmark)
        } else {
            None
        }
    }

    /// Resolve the fragmentation parameters to apply to relayed ClientHellos:
    /// the preset benchmark's temporary override when set, otherwise the
    /// process config. Returns (num_fragment, fragment_sleep_ms).
    fn current_frag(&self) -> (usize, u64) {
        if let Some(over) = *self.frag_override.read() {
            return over;
        }
        (self.config.num_fragment, self.config.fragment_sleep_ms)
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
        let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
        let udp_socket = match tun::bind_udp(bind_addr, self.fwmark()).await {
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
        let tun = Arc::clone(&self.tun);
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
                                        // Keep the split tunnel routing the voice
                                        // endpoint even if it left the DoH pool.
                                        tun.record_extra_ip(target_ip);
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

        // Discord via upstream SOCKS5 relay (see connect_and_relay).
        if self.config.relay_socks5.is_some() && DiscordManager::is_discord_domain(host) {
            match self.connect_via_relay(host, port).await {
                Ok((backend, relay_ip)) => {
                    println!(
                        "[{}] [RELAY] {}:{} via {}",
                        now_iso(),
                        host,
                        port,
                        relay_ip
                    );
                    let resp =
                        b"HTTP/1.1 200 Connection established\r\nProxy-agent: VegordProxy/3.0\r\n\r\n";
                    if client.write_all(resp).await.is_ok() {
                        self.relay_bidirectional(client, backend, host, port, &[relay_ip], true)
                            .await;
                    }
                }
                Err(e) => {
                    self.stats.conn_filtered.fetch_add(1, Ordering::Relaxed);
                    println!("[{}] [RELAY ERR] {}:{} - {}", now_iso(), host, port, e);
                    let _ = client
                        .write_all(b"HTTP/1.1 502 Bad Gateway\r\nProxy-agent: VegordProxy/3.0\r\n\r\n")
                        .await;
                }
            }
            if is_gateway {
                self.doh.note_gateway_disconnect();
            }
            return;
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
            let ok = client.write_all(resp).await.is_ok();
            if ok && self.config.tls_mitm && DiscordManager::is_discord_domain(host) {
                println!("[{}] [MITM] {} local TLS termination", now_iso(), host);
                mitm::spawn(
                    client,
                    backend,
                    host.to_string(),
                    self.current_frag(),
                    Arc::clone(&self.mitm),
                    self.config.connect_deadline_sec,
                    self.config.bulk_transfer_deadline_sec,
                    self.config.relay_idle_timeout_sec,
                );
            } else if ok {
                self.relay_bidirectional(client, backend, host, port, &targets, false)
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

        // With the split tunnel active the connect must be marked so it bypasses
        // the Discord TUN routes, and the target is recorded for route sync.
        self.tun.record_extra_ip(ip);
        let fwmark = self.fwmark();
        let stream_res = timeout(
            Duration::from_secs(timeout_sec),
            tun::connect_tcp(addr, fwmark),
        )
        .await;
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

        // Discord domains are tunneled through the upstream SOCKS5 relay when one
        // is configured: the relay's clean egress reaches the real Discord edge
        // (the ISP's Cloudflare-Spectrum path is rejected with 1034). No ClientHello
        // fragmentation is needed inside the tunnel — the relay reassembles it.
        if self.config.relay_socks5.is_some() && DiscordManager::is_discord_domain(host) {
            match self.connect_via_relay(host, port).await {
                Ok((backend, relay_ip)) => {
                    println!(
                        "[{}] [RELAY] {}:{} via {}",
                        now_iso(),
                        host,
                        port,
                        relay_ip
                    );
                    let resp = b"\x05\x00\x00\x01\x00\x00\x00\x00\x00\x00";
                    if client.write_all(resp).await.is_ok() {
                        self.relay_bidirectional(client, backend, host, port, &[relay_ip], true)
                            .await;
                    }
                }
                Err(e) => {
                    self.stats.conn_filtered.fetch_add(1, Ordering::Relaxed);
                    println!("[{}] [RELAY ERR] {}:{} - {}", now_iso(), host, port, e);
                    let _ = client
                        .write_all(b"\x05\x04\x00\x01\x00\x00\x00\x00\x00\x00")
                        .await;
                }
            }
            if is_gateway {
                self.doh.note_gateway_disconnect();
            }
            return;
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
            let ok = client.write_all(resp).await.is_ok();
            if ok && self.config.tls_mitm && DiscordManager::is_discord_domain(host) {
                println!("[{}] [MITM] {} local TLS termination", now_iso(), host);
                mitm::spawn(
                    client,
                    backend,
                    host.to_string(),
                    self.current_frag(),
                    Arc::clone(&self.mitm),
                    self.config.connect_deadline_sec,
                    self.config.bulk_transfer_deadline_sec,
                    self.config.relay_idle_timeout_sec,
                );
            } else if ok {
                self.relay_bidirectional(client, backend, host, port, &targets, false)
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

    /// Open a connection to the upstream SOCKS5 relay and issue CONNECT to
    /// `host:port`. Returns the tunneled stream plus the relay's resolved IP
    /// (for traffic accounting).
    async fn connect_via_relay(
        &self,
        host: &str,
        port: u16,
    ) -> std::io::Result<(TcpStream, IpAddr)> {
        let relay = self
            .config
            .relay_socks5
            .clone()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no relay configured"))?;
        let relay_ip = self
            .doh
            .query(&relay.host)
            .await
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("relay host {} not resolved", relay.host),
                )
            })?;
        let addr = SocketAddr::new(relay_ip, relay.port);
        let connect = timeout(
            Duration::from_secs(self.config.connect_deadline_sec),
            tun::connect_tcp(addr, self.fwmark()),
        )
        .await
        .map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::TimedOut, "relay connect timeout")
        })??;
        let mut connect = connect;
        connect.set_nodelay(true)?;
        self.socks5_connect(&mut connect, &relay, host, port).await?;
        Ok((connect, relay_ip))
    }

    /// Perform the SOCKS5 negotiation (greeting + optional username/password
    /// auth) and a CONNECT request for `host:port`. Domain names are passed
    /// through so the relay resolves them from its own clean network.
    async fn socks5_connect(
        &self,
        stream: &mut TcpStream,
        relay: &RelayConfig,
        host: &str,
        port: u16,
    ) -> std::io::Result<()> {
        let hs_timeout = Duration::from_secs(self.config.socket_timeout_sec);
        let mut buf = [0u8; 512];

        // Greeting
        let methods: &[u8] = if relay.user.is_some() {
            &[0x05, 0x02, 0x00, 0x02]
        } else {
            &[0x05, 0x01, 0x00]
        };
        stream.write_all(methods).await?;
        timeout(hs_timeout, stream.read_exact(&mut buf[..2])).await??;
        if buf[0] != 0x05 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "relay returned non-SOCKS5 version",
            ));
        }
        match buf[1] {
            0x00 => {}
            0x02 => {
                let user = relay.user.clone().unwrap_or_default();
                let pass = relay.pass.clone().unwrap_or_default();
                let mut auth = Vec::with_capacity(3 + user.len() + pass.len());
                auth.push(0x01);
                auth.push(user.len() as u8);
                auth.extend_from_slice(user.as_bytes());
                auth.push(pass.len() as u8);
                auth.extend_from_slice(pass.as_bytes());
                stream.write_all(&auth).await?;
                timeout(hs_timeout, stream.read_exact(&mut buf[..2])).await??;
                if buf[1] != 0x00 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "relay authentication failed",
                    ));
                }
            }
            m => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!("relay refused auth method 0x{:02x}", m),
                ));
            }
        }

        // CONNECT host:port (ATYP 0x03 = domain)
        let host_bytes = host.as_bytes();
        if host_bytes.len() > 255 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "hostname too long for SOCKS5",
            ));
        }
        let mut req = Vec::with_capacity(7 + host_bytes.len());
        req.extend_from_slice(&[0x05, 0x01, 0x00, 0x03, host_bytes.len() as u8]);
        req.extend_from_slice(host_bytes);
        req.extend_from_slice(&port.to_be_bytes());
        stream.write_all(&req).await?;

        timeout(hs_timeout, stream.read_exact(&mut buf[..4])).await??;
        if buf[0] != 0x05 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "relay returned non-SOCKS5 version",
            ));
        }
        if buf[1] != 0x00 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                format!("relay CONNECT failed (reply 0x{:02x})", buf[1]),
            ));
        }
        // Consume the remaining bind address bytes.
        let atyp = buf[3];
        let rest = match atyp {
            0x01 => 4,
            0x03 => {
                timeout(hs_timeout, stream.read_exact(&mut buf[..1])).await??;
                buf[0] as usize
            }
            0x04 => 16,
            _ => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "relay returned unknown address type",
                ));
            }
        };
        let mut sink = [0u8; 256];
        timeout(hs_timeout, stream.read_exact(&mut sink[..rest])).await??;
        // Consume the 2-byte BND.PORT that always follows the bind address.
        timeout(hs_timeout, stream.read_exact(&mut buf[..2])).await??;
        Ok(())
    }

    async fn relay_bidirectional(
        &self,
        mut client: TcpStream,
        mut backend: TcpStream,
        host: &str,
        port: u16,
        targets: &[IpAddr],
        via_relay: bool,
    ) {
        // Read first payload from client (TLS Client Hello)
        let mut first_buf = [0u8; 16384];
        let read_timeout = Duration::from_secs(self.config.socket_timeout_sec);

        let first_n = match timeout(read_timeout, client.read(&mut first_buf)).await {
            Ok(Ok(n)) if n > 0 => n,
            _ => return,
        };

        // Relay path: the tunnel is already established and unfiltered, so just
        // forward the whole ClientHello (no fragmentation, no IP retry) and wait
        // for the ServerHello before entering the bidirectional copy loop.
        if via_relay {
            if backend.write_all(&first_buf[..first_n]).await.is_err() {
                return;
            }
            let is_tls = first_buf[0] == 0x16 && first_n >= 6 && first_buf[5] == 0x01;
            if is_tls {
                let handshake_timeout =
                    Duration::from_secs(self.config.relay_handshake_timeout_sec);
                let mut hello = [0u8; 8192];
                match timeout(handshake_timeout, backend.read(&mut hello)).await {
                    Ok(Ok(n)) if n > 0 => {
                        if client.write_all(&hello[..n]).await.is_err() {
                            return;
                        }
                        self.stats.record_dl(targets[0], n as u64);
                    }
                    _ => return,
                }
            }
            let ip = targets[0];
            self.stats.record_ul(ip, first_n as u64);
            self.relay_copy_loop(client, backend, host, port, ip).await;
            return;
        }

        // Send the ClientHello. TLS SNI fragmentation is the anti-SNI-filter
        // technique, but it must be applied ONLY to Discord SNIs: the ISP DPI
        // resets unfragmented Discord handshakes, yet a fragmented handshake to
        // ANY other host (DoH resolvers, google, etc.) is itself reset by the
        // same filter. Every non-Discord host therefore goes out in a single
        // segment. If the connection is reset mid-handshake (GFW RST against
        // Cloudflare etc.), the client has not received anything yet — it is
        // still waiting for the ServerHello — so we can transparently reconnect
        // to the same or an alternate IP and resend the exact same ClientHello.
        let fragment_tls = DiscordManager::is_discord_domain(host);
        let mut ip_idx = 0usize;
        let max_relay_attempts = self.config.relay_retries.max(1);
        let mut delivered = false;
        let frag = self.current_frag();
        for attempt in 0..max_relay_attempts {
            if attempt > 0 {
                // GFW RSTs tend to land in short bursts that hit every attempt
                // within the same window, so space retries out a bit.
                tokio::time::sleep(Duration::from_millis(self.config.relay_retry_sleep_ms)).await;
                if targets.len() > 1 {
                    ip_idx = (ip_idx + 1) % targets.len();
                }
                let ip = targets[ip_idx];
                println!(
                    "[{}] [{}] {} via {}",
                    now_iso(),
                    if fragment_tls { "FRAG RETRY" } else { "RETRY" },
                    host,
                    ip
                );
                match self.connect_target(ip, port).await {
                    Ok(stream) => backend = stream,
                    Err(_) => continue,
                }
            }

            let send_result = if fragment_tls {
                send_fragmented_async(
                    &mut backend,
                    &first_buf[..first_n],
                    frag.0,
                    frag.1,
                )
                .await
            } else {
                async {
                    backend.write_all(&first_buf[..first_n]).await?;
                    backend.flush().await?;
                    Ok(())
                }
                .await
            };
            match send_result {
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
        self.relay_copy_loop(client, backend, host, port, ip).await;
    }

    /// Bi-directional async forwarding, half-close aware (spec section 5.1).
    ///
    /// The old code used `tokio::select!`, which cancelled the other task the
    /// moment either direction finished — dropping buffered upload bytes when
    /// a server sent an early response / 100-continue. Instead each direction
    /// runs to completion independently (tokio::join!). When one direction
    /// hits EOF it performs a real TCP half-close (shutdown) on the opposite
    /// write half, and the connection is only torn down once BOTH directions
    /// have finished. The relay is force-closed only by the bulk-transfer
    /// deadline (overall ceiling) or the relay_idle_timeout_sec idle timeout
    /// (zero bytes in either direction), both of which are logged so a silently
    /// failed CDN upload is diagnosable.
    async fn relay_copy_loop(
        &self,
        mut client: TcpStream,
        mut backend: TcpStream,
        host: &str,
        port: u16,
        ip: IpAddr,
    ) {
        let (mut cr, mut cw) = client.split();
        let (mut br, mut bw) = backend.split();

        let stats_ul = Arc::clone(&self.stats);
        let stats_dl = Arc::clone(&self.stats);

        // Idle timeout: wall-clock of the last byte moved in EITHER direction.
        // Fires only when the whole connection has been silent for
        // relay_idle_timeout_sec — a slow upload keeps resetting it, so it is
        // never killed; a connection whose peer silently vanished (GFW drop
        // with no RST, half-dead CDN mid-upload) is reaped and logged instead
        // of hanging the app until the bulk-transfer deadline.
        let idle_limit = Duration::from_secs(self.config.relay_idle_timeout_sec.max(1));
        let last_activity = Arc::new(std::sync::atomic::AtomicU64::new(now_ms()));
        let last_activity_ul = Arc::clone(&last_activity);
        let last_activity_dl = Arc::clone(&last_activity);

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
                            println!(
                                "[{}] [RELAY CLOSE] {}:{} app->backend write failed",
                                now_iso(),
                                host,
                                port
                            );
                            break;
                        }
                        stats_ul.record_ul(ip, n as u64);
                        last_activity_ul.store(now_ms(), Ordering::Relaxed);
                    }
                    Err(e) => {
                        println!(
                            "[{}] [RELAY CLOSE] {}:{} app->backend read error: {}",
                            now_iso(),
                            host,
                            port,
                            e
                        );
                        break;
                    }
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
                            println!(
                                "[{}] [RELAY CLOSE] {}:{} backend->app write failed",
                                now_iso(),
                                host,
                                port
                            );
                            break;
                        }
                        stats_dl.record_dl(ip, n as u64);
                        last_activity_dl.store(now_ms(), Ordering::Relaxed);
                    }
                    Err(e) => {
                        println!(
                            "[{}] [RELAY CLOSE] {}:{} backend->app read error: {}",
                            now_iso(),
                            host,
                            port,
                            e
                        );
                        break;
                    }
                }
            }
        };

        let deadline = tokio::time::sleep(Duration::from_secs(self.config.bulk_transfer_deadline_sec));
        tokio::pin!(deadline);
        let mut idle_sleep = Box::pin(tokio::time::sleep(idle_limit));
        let relay = async {
            tokio::join!(client_to_backend, backend_to_client);
        };
        tokio::pin!(relay);

        loop {
            tokio::select! {
                _ = &mut deadline => {
                    println!(
                        "[{}] [RELAY DEADLINE] {}:{} exceeded {}s, closing",
                        now_iso(),
                        host,
                        port,
                        self.config.bulk_transfer_deadline_sec
                    );
                    break;
                }
                _ = &mut idle_sleep => {
                    let last = last_activity.load(Ordering::Relaxed);
                    if now_ms().saturating_sub(last) >= idle_limit.as_millis() as u64 {
                        println!(
                            "[{}] [RELAY IDLE TIMEOUT] {}:{} no data in either direction for {}s, closing",
                            now_iso(),
                            host,
                            port,
                            idle_limit.as_secs()
                        );
                        break;
                    }
                    idle_sleep = Box::pin(tokio::time::sleep(idle_limit));
                }
                _ = &mut relay => break,
            }
        }
    }
}

/// Milliseconds since the UNIX epoch (wall clock), used for the relay idle
/// tracker so activity in one direction resets the timeout for the whole
/// connection.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
