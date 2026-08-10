use std::collections::HashSet;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::os::fd::AsRawFd;
use std::os::unix::io::RawFd;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use futures_util::TryStreamExt;
use parking_lot::RwLock;
use tokio::net::{TcpStream, UdpSocket};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use crate::config::Config;
use crate::discord::DiscordManager;
use crate::stats::now_iso;

/// TUN interface created by the tun2proxy child. Only traffic to routed Discord
/// IPs is captured; the default network is never touched.
#[allow(dead_code)]
pub const TUN_NAME: &str = "vegord0";
/// Firewall mark applied to every gfw_proxy outbound socket. Rule priority 100
/// sends marked traffic straight to the `main` table so the proxy's own
/// Discord connections bypass the TUN (loop avoidance).
#[allow(dead_code)]
pub const TUN_FWMARK: u32 = 0x54f;
/// Routing table holding the per-Discord-IP `/32` routes into the TUN. Consulted
/// by rule priority 200 before falling through to `main`.
#[allow(dead_code)]
pub const TUN_TABLE: u32 = 100;
const RT_TABLE_MAIN: u32 = 254;
const FWMARK_RULE_PRIO: u32 = 100;
const TABLE_RULE_PRIO: u32 = 200;
const CAP_NET_ADMIN: u64 = 1 << 12;

/// Set SO_MARK on a socket so the kernel routes it through the `main` table
/// (fwmark rule) instead of the Discord TUN table (loop avoidance).
pub fn set_fwmark(fd: RawFd, mark: u32) -> io::Result<()> {
    let ret = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_MARK,
            (&mark as *const u32).cast(),
            std::mem::size_of::<u32>() as libc::socklen_t,
        )
    };
    if ret != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Async TCP connect that (optionally) stamps the socket with the fwmark before
/// connecting, so the connection bypasses the split-tunnel routes. Returns the
/// connected stream; callers keep their own deadline/timeout wrapper.
pub async fn connect_tcp(addr: SocketAddr, fwmark: Option<u32>) -> io::Result<TcpStream> {
    use tokio::io::Interest;

    let domain = if addr.is_ipv4() {
        socket2::Domain::IPV4
    } else {
        socket2::Domain::IPV6
    };
    let socket = socket2::Socket::new(domain, socket2::Type::STREAM, Some(socket2::Protocol::TCP))?;
    if let Some(mark) = fwmark {
        set_fwmark(socket.as_raw_fd(), mark)?;
    }
    socket.set_nonblocking(true)?;
    match socket.connect(&addr.into()) {
        Ok(()) => {}
        Err(e) => {
            // Non-blocking connect: the kernel reports EINPROGRESS (or EAGAIN)
            // while the connection is being established. Older Rust maps this
            // to WouldBlock, newer toolchains to the dedicated InProgress kind,
            // so match on the raw errno instead. Only genuine failures (RST,
            // unreachable, ...) are errors — completion is detected below via
            // writability + SO_ERROR.
            let raw = e.raw_os_error();
            let in_progress = raw == Some(libc::EINPROGRESS)
                || raw == Some(libc::EAGAIN)
                || e.kind() == io::ErrorKind::WouldBlock;
            if !in_progress {
                return Err(e);
            }
        }
    }
    let std_stream: std::net::TcpStream = socket.into();
    let stream = TcpStream::from_std(std_stream)?;
    // A non-blocking connect completes asynchronously: wait for writability,
    // then surface the connect result (RST/refused -> real error) like a
    // blocking connect would.
    stream.ready(Interest::WRITABLE).await?;
    if let Some(err) = stream.take_error()? {
        return Err(err);
    }
    Ok(stream)
}

/// Async UDP socket bound to `addr`, optionally stamped with the fwmark so the
/// relayed voice path (and everything it sends) bypasses the TUN.
pub async fn bind_udp(addr: SocketAddr, fwmark: Option<u32>) -> io::Result<UdpSocket> {
    let domain = if addr.is_ipv4() {
        socket2::Domain::IPV4
    } else {
        socket2::Domain::IPV6
    };
    let socket = socket2::Socket::new(domain, socket2::Type::DGRAM, Some(socket2::Protocol::UDP))?;
    if let Some(mark) = fwmark {
        set_fwmark(socket.as_raw_fd(), mark)?;
    }
    socket.set_nonblocking(true)?;
    socket.bind(&addr.into())?;
    let std_udp: std::net::UdpSocket = socket.into();
    UdpSocket::from_std(std_udp)
}

fn has_cap_net_admin() -> bool {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return false;
    };
    for line in status.lines() {
        if let Some(hex) = line.strip_prefix("CapEff:") {
            if let Ok(val) = u64::from_str_radix(hex.trim(), 16) {
                return (val & CAP_NET_ADMIN) != 0;
            }
        }
    }
    false
}

/// After a SIGKILL the tun2proxy child can be orphaned while still holding the
/// `vegord0` device. Best-effort reap: kill any tun2proxy process of ours that
/// references our TUN name.
fn kill_orphan_tun2proxy(tun_name: &str) {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<i32>() else {
            continue;
        };
        let Ok(cmdline) = std::fs::read(format!("/proc/{pid}/cmdline")) else {
            continue;
        };
        let cmd: String = cmdline
            .iter()
            .map(|&b| if b == 0 { ' ' } else { b as char })
            .collect();
        if cmd.contains("tun2proxy") && cmd.contains(tun_name) {
            println!("[{}] [TUN] killing orphaned tun2proxy pid {}", now_iso(), pid);
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
    }
}

async fn new_netlink() -> Option<rtnetlink::Handle> {
    let (connection, handle, _) = rtnetlink::new_connection().ok()?;
    tokio::spawn(connection);
    Some(handle)
}

async fn link_index(name: &str) -> io::Result<u32> {
    let handle = new_netlink()
        .await
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "netlink connection failed"))?;
    let mut links = handle
        .link()
        .get()
        .match_name(name.to_string())
        .execute();
    match links.try_next().await {
        Ok(Some(link)) => Ok(link.header.index),
        Ok(None) => Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("interface {name} not found"),
        )),
        Err(e) => Err(io::Error::new(io::ErrorKind::Other, e.to_string())),
    }
}

/// `ip rule add fwmark <mark> lookup main pref <prio>` — marked gfw_proxy
/// sockets bypass the TUN table.
async fn ensure_fwmark_rule(handle: &rtnetlink::Handle, fwmark: u32, prio: u32) {
    let mut req = handle
        .rule()
        .add()
        .v4()
        .fw_mark(fwmark)
        .table_id(RT_TABLE_MAIN)
        .priority(prio)
        .action(rtnetlink::packet_route::rule::RuleAction::ToTable)
        .replace();
    // Match the whole mark (default `ip rule` behaviour when only `fwmark` is
    // given).
    req.message_mut()
        .attributes
        .push(rtnetlink::packet_route::rule::RuleAttribute::FwMask(u32::MAX));
    let _ = req.execute().await;
}

/// `ip rule add lookup <table> pref <prio>` — Discord IP routes in the custom
/// table are consulted before `main`.
async fn ensure_table_rule(handle: &rtnetlink::Handle, table: u32, prio: u32) {
    let _ = handle
        .rule()
        .add()
        .v4()
        .table_id(table)
        .priority(prio)
        .action(rtnetlink::packet_route::rule::RuleAction::ToTable)
        .replace()
        .execute()
        .await;
}

async fn del_matching_rules(fwmark: u32, table: u32) {
    let Some(handle) = new_netlink().await else {
        return;
    };
    let mut rules = handle.rule().get(rtnetlink::IpVersion::V4).execute();
    while let Some(rule) = rules.try_next().await.ok().flatten() {
        let has_prio = rule
            .attributes
            .iter()
            .any(|a| matches!(a, rtnetlink::packet_route::rule::RuleAttribute::Priority(p) if *p == FWMARK_RULE_PRIO || *p == TABLE_RULE_PRIO));
        let has_mark = rule.attributes.iter().any(|a| {
            matches!(a, rtnetlink::packet_route::rule::RuleAttribute::FwMark(m) if *m == fwmark)
        });
        let rule_table = rule.header.table as u32;
        // Delete our two rules: the fwmark bypass (mark + main) and the table
        // lookup (table + no mark).
        let ours = (has_mark && rule_table == RT_TABLE_MAIN)
            || (!has_mark && has_prio && rule_table == table);
        if ours {
            let _ = handle.rule().del(rule).execute().await;
        }
    }
}

async fn ensure_route(ip: IpAddr, table: u32, ifindex: u32) {
    let Some(handle) = new_netlink().await else {
        return;
    };
    let IpAddr::V4(v4) = ip else {
        return; // TUN is IPv4-only; IPv6 Discord traffic stays on main.
    };
    let route = rtnetlink::RouteMessageBuilder::<std::net::Ipv4Addr>::new()
        .destination_prefix(v4, 32)
        .table_id(table)
        .output_interface(ifindex)
        .build();
    let _ = handle.route().add(route).replace().execute().await;
}

/// Remove every route in the split-tunnel table (crash recovery).
async fn flush_routes(table: u32) {
    let Some(handle) = new_netlink().await else {
        return;
    };
    let msg = rtnetlink::RouteMessageBuilder::<std::net::Ipv4Addr>::new()
        .table_id(table)
        .build();
    let mut routes = handle.route().get(msg).execute();
    while let Some(route) = routes.try_next().await.ok().flatten() {
        let _ = handle.route().del(route).execute().await;
    }
}

pub struct TunManager {
    pub enabled: bool,
    tun_name: String,
    fwmark: u32,
    table: u32,
    tun2proxy_bin: PathBuf,
    child: Mutex<Option<Child>>,
    running: AtomicBool,
    ifindex: RwLock<Option<u32>>,
    route_count: AtomicUsize,
    extra_ips: RwLock<HashSet<IpAddr>>,
}

impl TunManager {
    pub fn new(config: &Config) -> Arc<Self> {
        Arc::new(Self {
            enabled: config.tun_split_enabled,
            tun_name: config.tun_name.clone(),
            fwmark: config.tun_fwmark,
            table: config.tun_table,
            tun2proxy_bin: PathBuf::from(&config.tun2proxy_bin),
            child: Mutex::new(None),
            running: AtomicBool::new(false),
            ifindex: RwLock::new(None),
            route_count: AtomicUsize::new(0),
            extra_ips: RwLock::new(HashSet::new()),
        })
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    pub fn route_count(&self) -> usize {
        self.route_count.load(Ordering::Relaxed)
    }

    /// Register a Discord target IP seen on a live connection so the reconcile
    /// loop routes it into the TUN even if it left the DoH pool.
    pub fn record_extra_ip(&self, ip: IpAddr) {
        if ip.is_loopback() {
            return;
        }
        self.extra_ips.write().insert(ip);
    }

    pub async fn start(self: Arc<Self>, discord: Arc<DiscordManager>, proxy_port: u16) {
        if !self.enabled {
            return;
        }
        println!("[{}] [TUN] split-tunnel requested, probing CAP_NET_ADMIN", now_iso());
        if !has_cap_net_admin() {
            println!(
                "[{}] [TUN] missing CAP_NET_ADMIN (run: setcap cap_net_admin,cap_net_raw+ep on the gfw_proxy binary). Split tunnel disabled.",
                now_iso()
            );
            return;
        }

        // Crash recovery: reap orphaned tun2proxy + drop stale rules/routes.
        kill_orphan_tun2proxy(&self.tun_name);
        del_matching_rules(self.fwmark, self.table).await;
        flush_routes(self.table).await;
        println!("[{}] [TUN] stale tun2proxy/rules cleared", now_iso());

        // Spawn tun2proxy: creates the vegord0 device and relays it into our
        // SOCKS5 entry point (127.0.0.1:<proxy_port>). No --setup: routing is
        // managed here and only for Discord IPs.
        let proxy_url = format!("socks5://127.0.0.1:{}", proxy_port);
        let mut child = match Command::new(&self.tun2proxy_bin)
            .arg("--tun")
            .arg(&self.tun_name)
            .arg("--proxy")
            .arg(&proxy_url)
            .arg("--mtu")
            .arg("1500")
            .arg("--udp-timeout")
            .arg("30")
            .arg("--verbosity")
            .arg("warn")
            .arg("--exit-on-fatal-error")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                println!("[{}] [TUN] failed to spawn tun2proxy at {}: {}", now_iso(), self.tun2proxy_bin.display(), e);
                return;
            }
        };
        println!("[{}] [TUN] tun2proxy spawned ({} -> {})", now_iso(), self.tun2proxy_bin.display(), proxy_url);

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        if let Some(out) = stdout {
            tokio::spawn(async move {
                use tokio::io::{AsyncBufReadExt, BufReader};
                let mut lines = BufReader::new(out).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    println!("[{}] [TUN] {}", now_iso(), line);
                }
            });
        }
        if let Some(err) = stderr {
            tokio::spawn(async move {
                use tokio::io::{AsyncBufReadExt, BufReader};
                let mut lines = BufReader::new(err).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    println!("[{}] [TUN] {}", now_iso(), line);
                }
            });
        }

        *self.child.lock().await = Some(child);
        self.running.store(true, Ordering::Relaxed);

        // Wait for the TUN link to appear (tun2proxy creates it in general_run).
        let mut ifindex = None;
        for _ in 0..50 {
            match link_index(&self.tun_name).await {
                Ok(idx) => {
                    ifindex = Some(idx);
                    break;
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(200)).await,
            }
        }
        let Some(ifindex) = ifindex else {
            println!("[{}] [TUN] {} did not come up in time", now_iso(), self.tun_name);
            self.stop().await;
            return;
        };
        *self.ifindex.write() = Some(ifindex);
        println!("[{}] [TUN] {} up (ifindex {})", now_iso(), self.tun_name, ifindex);

        // Install the routing rules.
        let Some(handle) = new_netlink().await else {
            println!("[{}] [TUN] netlink unavailable", now_iso());
            self.stop().await;
            return;
        };
        ensure_fwmark_rule(&handle, self.fwmark, FWMARK_RULE_PRIO).await;
        ensure_table_rule(&handle, self.table, TABLE_RULE_PRIO).await;
        drop(handle);
        println!(
            "[{}] [TUN] ip rules installed (fwmark {} -> main, lookup table {})",
            now_iso(),
            self.fwmark,
            self.table
        );

        // Reconcile loop: keep the TUN routing every known Discord IP.
        let me = Arc::clone(&self);
        let discord_r = Arc::clone(&discord);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(3));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                if !me.is_running() {
                    return;
                }
                me.reconcile_routes(&discord_r).await;
            }
        });

        // Watch the child: on unexpected exit, tear down the tunnel state so
        // Discord is not left pointed at a dead device. Polls try_wait so it
        // never holds the child lock across an await (which would deadlock
        // stop()).
        let me = Arc::clone(&self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(500)).await;
                let status = {
                    let mut guard = me.child.lock().await;
                    match guard.as_mut() {
                        Some(c) => c.try_wait().ok().flatten(),
                        None => break,
                    }
                };
                if let Some(status) = status {
                    println!(
                        "[{}] [TUN] tun2proxy exited ({:?}), tearing down split tunnel",
                        now_iso(),
                        status.code()
                    );
                    me.teardown_routes().await;
                    me.running.store(false, Ordering::Relaxed);
                    *me.child.lock().await = None;
                    return;
                }
            }
        });
    }

    async fn reconcile_routes(&self, discord: &DiscordManager) {
        let Some(ifindex) = *self.ifindex.read() else {
            return;
        };
        let mut ips: HashSet<IpAddr> = self.extra_ips.read().iter().copied().collect();
        for (ip, _, _) in discord.get_ips_snapshot() {
            ips.insert(ip);
        }
        let mut added = 0usize;
        for ip in ips {
            if !ip.is_ipv4() {
                continue;
            }
            ensure_route(ip, self.table, ifindex).await;
            added += 1;
        }
        let prev = self.route_count.swap(added, Ordering::Relaxed);
        if prev != added {
            println!(
                "[{}] [TUN] routing {} Discord IP(s) via {}",
                now_iso(),
                added,
                self.tun_name
            );
        }
    }

    async fn teardown_routes(&self) {
        del_matching_rules(self.fwmark, self.table).await;
        flush_routes(self.table).await;
        self.route_count.store(0, Ordering::Relaxed);
    }

    pub async fn stop(&self) {
        if !self.running.swap(false, Ordering::Relaxed) {
            return;
        }
        let mut guard = self.child.lock().await;
        if let Some(mut child) = guard.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        drop(guard);
        self.teardown_routes().await;
        println!("[{}] [TUN] split tunnel stopped", now_iso());
    }
}
