use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

/// Upstream SOCKS5 relay for SNI-filtered domains (e.g. Discord). Connections
/// to relay-routed hosts are tunneled through this proxy instead of connecting
/// directly, so the relay's clean egress reaches the origin without the ISP's
/// DNS-hijack/Cloudflare-Spectrum path (which Cloudflare rejects with 1034).
#[derive(Debug, Clone)]
pub struct RelayConfig {
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub pass: Option<String>,
}

/// Parse a relay spec of the form `[user:pass@]host:port`.
pub fn parse_relay_socks5(spec: &str) -> Option<RelayConfig> {
    let spec = spec.trim();
    let (auth, hostport) = match spec.rsplit_once('@') {
        Some((a, hp)) => (Some(a), hp),
        None => (None, spec),
    };
    let (host, port) = hostport.rsplit_once(':')?;
    let host = host.trim().to_string();
    if host.is_empty() {
        return None;
    }
    let port: u16 = port.parse().ok()?;
    let (user, pass) = match auth {
        Some(a) => match a.split_once(':') {
            Some((u, p)) => (Some(u.to_string()), Some(p.to_string())),
            None => (Some(a.to_string()), None),
        },
        None => (None, None),
    };
    Some(RelayConfig {
        host,
        port,
        user,
        pass,
    })
}

#[derive(Debug, Clone)]
pub struct Config {
    pub listen_port: u16,
    pub num_fragment: usize,
    pub fragment_sleep_ms: u64,
    pub log_every_sec: u64,
    // Raw-IP DoH servers (e.g. https://1.1.1.1, https://8.8.8.8) present a
    // certificate whose CN/SAN is the IP literal, which reqwest's default
    // verifier accepts. Some resolvers do not ship an IP SAN though, so this
    // stays configurable. It is scoped to the DoH client only and defaults to
    // true because a failing DoH handshake takes down ALL DNS.
    pub allow_insecure: bool,
    pub socket_timeout_sec: u64,
    pub voice_socket_timeout_sec: u64,
    pub connect_retries: u32,
    pub relay_retries: u32,
    pub relay_retry_sleep_ms: u64,
    pub relay_handshake_timeout_sec: u64,
    // Phase-specific deadlines (spec section 5.3): the connect timeout, the
    // handshake timeout and the idle timeout must NOT be conflated, and the
    // bulk-transfer deadline is an overall ceiling, not an idle ceiling.
    pub connect_deadline_sec: u64,
    pub bulk_transfer_deadline_sec: u64,
    // Idle timeout for the steady-state relay: fires only when ZERO bytes have
    // moved in either direction for N seconds (a truly stuck connection, e.g. a
    // silently dropped upstream during a CDN upload, or a peer that vanished
    // without RST/EOF). Slow-but-alive transfers keep making progress and are
    // never killed by this; it only reaps genuinely dead connections that would
    // otherwise hang until the bulk-transfer deadline.
    pub relay_idle_timeout_sec: u64,
    pub doh_max_retries: usize,
    pub doh_max_fails_before_switch: u32,
    pub doh_blacklist_sec: u64,
    pub doh_timeout_sec: u64,
    pub discord_ping_interval_sec: u64,
    pub discord_ping_timeout_sec: u64,
    pub discord_max_ips: usize,
    pub discord_min_rtt_ms: f64,
    pub data_dir: PathBuf,
    pub control_port: u16,
    pub control_token: String,
    pub doh_probe_attempts: usize,
    pub doh_probe_timeout_sec: u64,
    pub doh_probe_concurrency: usize,
    pub doh_min_rescan_interval_sec: u64,
    pub doh_reconnect_window_sec: u64,
    pub doh_switch_margin_ms: f64,
    // Voice UDP relay health check (spec section 5.2): heartbeat + failover.
    pub udp_heartbeat_sec: u64,
    pub udp_loss_window_sec: u64,
    // Preset / ISP-awareness system (spec section 4).
    pub preset_sync_enabled: bool,
    pub panel_base_url: String,
    pub panel_timeout_sec: u64,
    // Consent-gated preset upload: empty disables upload entirely.
    pub panel_upload_token: String,
    // Preferred DoH resolver index into DOH_SERVERS, seeded from the last saved
    // preset's doh_resolvers_ranked so a known-good server (learned last run)
    // is tried first on startup instead of always starting at index 0.
    // None = default behavior (start at DOH_SERVERS[0]).
    pub preferred_doh_index: Option<usize>,
    // Upstream SOCKS5 relay for Discord (bypasses the Cloudflare-Spectrum dead
    // end). None = legacy direct/offline-DNS path.
    pub relay_socks5: Option<RelayConfig>,
    // Local TLS MITM for Discord hosts: the proxy terminates the app's TLS
    // with a self-signed cert for the target host (the app must run with
    // --ignore-certificate-errors) and re-connects upstream with its own rustls
    // stack. This is the only way past ISPs that fingerprint BoringSSL: the
    // app's BoringSSL ClientHello never leaves loopback, and the proxy's rustls
    // ClientHello (fragmented) passes the DPI. Off by default.
    pub tls_mitm: bool,
    // Split-tunnel: tun2proxy relays a TUN device (which routes only Discord
    // IPs) into our own SOCKS5 entry point. Off by default; requires
    // CAP_NET_ADMIN on the gfw_proxy binary (setcap at install).
    pub tun_split_enabled: bool,
    pub tun_name: String,
    pub tun_fwmark: u32,
    pub tun_table: u32,
    pub tun2proxy_bin: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen_port: 4500,
            num_fragment: 6,
            fragment_sleep_ms: 1,
            log_every_sec: 30,
            allow_insecure: true,
            socket_timeout_sec: 8,
            voice_socket_timeout_sec: 120,
            connect_retries: 3,
            relay_retries: 3,
            relay_retry_sleep_ms: 400,
            relay_handshake_timeout_sec: 2,
            connect_deadline_sec: 10,
            bulk_transfer_deadline_sec: 600,
            relay_idle_timeout_sec: 120,
            doh_max_retries: 5,
            doh_max_fails_before_switch: 3,
            doh_blacklist_sec: 300,
            doh_timeout_sec: 8,
            discord_ping_interval_sec: 30,
            discord_ping_timeout_sec: 3,
            discord_max_ips: 20,
            discord_min_rtt_ms: 0.0,
            data_dir: PathBuf::from("."),
            control_port: 4501,
            control_token: String::new(),
            doh_probe_attempts: 2,
            doh_probe_timeout_sec: 4,
            doh_probe_concurrency: 8,
            doh_min_rescan_interval_sec: 300,
            doh_reconnect_window_sec: 120,
            doh_switch_margin_ms: 50.0,
            udp_heartbeat_sec: 3,
            udp_loss_window_sec: 15,
            preset_sync_enabled: true,
            panel_base_url: "https://vergoboy.ir/vegord/api/v1".to_string(),
            panel_timeout_sec: 6,
            panel_upload_token: String::new(),
            preferred_doh_index: None,
            relay_socks5: None,
            tls_mitm: false,
            tun_split_enabled: false,
            tun_name: "vegord0".to_string(),
            tun_fwmark: 0x54f,
            tun_table: 100,
            tun2proxy_bin: "tun2proxy-bin".to_string(),
        }
    }
}

pub static DOH_SERVERS: &[&str] = &[
    "https://cloudflare-dns.com/dns-query?dns=",
    "https://1.0.0.1/dns-query?dns=",
    "https://1.1.1.1/dns-query?dns=",
    "https://8.8.8.8/dns-query?dns=",
    "https://8.8.4.4/dns-query?dns=",
    "https://dns.google/dns-query?dns=",
    "https://doh.opendns.com/dns-query?dns=",
    "https://208.67.222.222/dns-query?dns=",
];

pub static DISCORD_DOMAINS: &[&str] = &[
    "discord.com",
    "discord.gg",
    "discordapp.com",
    "discordapp.net",
    "discord.media",
    "discordstatus.com",
    "gateway.discord.gg",
    "gateway-us-east1-b.discord.gg",
    "cdn.discordapp.com",
    "cdn.discord.com",
    "media.discordapp.net",
    "media.discord.com",
    "status.discord.com",
    "api.discord.com",
];

pub fn get_offline_dns() -> &'static HashMap<&'static str, &'static str> {
    static OFFLINE_DNS: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    OFFLINE_DNS.get_or_init(|| {
        let mut m = HashMap::new();
        m.insert("cloudflare-dns.com", "203.32.120.226");
        // Discord domains are intentionally NOT pinned here: they must resolve via
        // DoH so the real Discord anycast IPs (162.159.x.2x2) land in the ping
        // pool and are routed directly with ClientHello fragmentation. Routing
        // them through the ISP's SNI-forwarding host (203.32.120.226) is rejected
        // with Cloudflare 1034 "Edge IP Restricted".
        m.insert("dns.google", "8.8.8.8");
        m.insert("doh.opendns.com", "208.67.222.222");
        m.insert("dns.quad9.net", "9.9.9.9");
        m.insert("doh.libredns.gr", "116.202.176.26");
        m.insert("dns.bitdefender.net", "34.84.232.67");
        m.insert("secure.avastdns.com", "185.185.133.66");
        m.insert("doh.cleanbrowsing.org", "185.228.168.9");
        m.insert("doh.dns.sb", "185.184.71.198");
        m.insert("doh.tiar.app", "5.9.52.91");
        m.insert("doh.dnswarden.com", "116.203.249.54");
        m.insert("doh.powerdns.org", "188.166.104.87");
        m.insert("dns.electrotm.org", "78.157.42.100");
        m.insert("cluster-1.gac.edu", "138.236.128.101");
        m.insert("dns.hostux.net", "185.121.177.177");
        m.insert("doh.securedns.eu", "146.185.167.43");
        m.insert("doh.ffmuc.net", "5.1.66.255");
        m.insert("dns.cmrg.net", "199.58.81.218");
        m.insert("doh.centraleu.pi-dns.com", "116.202.120.165");
        m.insert("doh.dns.live", "104.28.1.1");
        m.insert("dns.friendi.ca", "198.50.200.234");
        m.insert("doh.bortzmeyer.org", "193.70.85.187");
        m.insert("doh.airdns.org", "37.120.215.68");
        m.insert("dns.hyperpipe.surge.sh", "188.114.97.3");
        m.insert("dns.digitale-gesellschaft.ch", "185.95.218.42");
        m.insert("doh.ibk.pl", "194.181.253.3");
        m.insert("dns.rubyfish.io", "139.162.235.169");
        m.insert("doh.otk.ee", "95.216.224.92");
        m.insert("dns.joatmalatesta.net", "192.161.48.7");
        m.insert("doh.shecan.ir", "178.22.122.100");
        m.insert("api.twitter.com", "104.244.42.66");
        m.insert("twitter.com", "104.244.42.1");
        m.insert("pbs.twimg.com", "93.184.220.70");
        m.insert("abs-0.twimg.com", "104.244.43.131");
        m.insert("abs.twimg.com", "152.199.24.185");
        m.insert("video.twimg.com", "192.229.220.133");
        m.insert("t.co", "104.244.42.69");
        m.insert("ton.local.twitter.com", "104.244.42.1");
        m.insert("instagram.com", "163.70.128.174");
        m.insert("www.instagram.com", "163.70.128.174");
        m.insert("static.cdninstagram.com", "163.70.132.63");
        m.insert("scontent.cdninstagram.com", "163.70.132.63");
        m.insert("privacycenter.instagram.com", "163.70.128.174");
        m.insert("help.instagram.com", "163.70.128.174");
        m.insert("l.instagram.com", "163.70.128.174");
        m.insert("e1.whatsapp.net", "163.70.128.60");
        m.insert("e2.whatsapp.net", "163.70.128.60");
        m.insert("e3.whatsapp.net", "163.70.128.60");
        m.insert("e4.whatsapp.net", "163.70.128.60");
        m.insert("e5.whatsapp.net", "163.70.128.60");
        m.insert("e6.whatsapp.net", "163.70.128.60");
        m.insert("e7.whatsapp.net", "163.70.128.60");
        m.insert("e8.whatsapp.net", "163.70.128.60");
        m.insert("e9.whatsapp.net", "163.70.128.60");
        m.insert("e10.whatsapp.net", "163.70.128.60");
        m.insert("e11.whatsapp.net", "163.70.128.60");
        m.insert("e12.whatsapp.net", "163.70.128.60");
        m.insert("e13.whatsapp.net", "163.70.128.60");
        m.insert("e14.whatsapp.net", "163.70.128.60");
        m.insert("e15.whatsapp.net", "163.70.128.60");
        m.insert("e16.whatsapp.net", "163.70.128.60");
        m.insert("dit.whatsapp.net", "185.60.219.60");
        m.insert("g.whatsapp.net", "185.60.218.54");
        m.insert("wa.me", "185.60.219.60");
        m.insert("web.whatsapp.com", "31.13.83.51");
        m.insert("whatsapp.net", "31.13.83.51");
        m.insert("whatsapp.com", "31.13.83.51");
        m.insert("cdn.whatsapp.net", "31.13.83.51");
        m.insert("snr.whatsapp.net", "31.13.83.51");
        m.insert("static.xx.fbcdn.net", "31.13.75.13");
        m.insert("scontent-mct1-1.xx.fbcdn.net", "31.13.75.13");
        m.insert("video-mct1-1.xx.fbcdn.net", "31.13.75.13");
        m.insert("video.fevn1-2.fna.fbcdn.net", "185.48.241.146");
        m.insert("video.fevn1-4.fna.fbcdn.net", "185.48.243.145");
        m.insert("scontent.xx.fbcdn.net", "185.48.240.146");
        m.insert("scontent.fevn1-1.fna.fbcdn.net", "185.48.240.145");
        m.insert("scontent.fevn1-2.fna.fbcdn.net", "185.48.241.145");
        m.insert("scontent.fevn1-3.fna.fbcdn.net", "185.48.242.146");
        m.insert("scontent.fevn1-4.fna.fbcdn.net", "185.48.243.147");
        m.insert("connect.facebook.net", "31.13.84.51");
        m.insert("facebook.com", "31.13.65.49");
        m.insert("developers.facebook.com", "31.13.84.8");
        m.insert("about.meta.com", "163.70.128.13");
        m.insert("meta.com", "163.70.128.13");
        m.insert("ocsp.pki.goog", "172.217.16.195");
        m.insert("googleads.g.doubleclick.net", "45.157.177.108");
        m.insert("fonts.gstatic.com", "142.250.185.227");
        m.insert("rr2---sn-vh5ouxa-hju6.googlevideo.com", "213.202.6.141");
        m.insert("jnn-pa.googleapis.com", "45.157.177.108");
        m.insert("static.doubleclick.net", "202.61.195.218");
        m.insert("rr4---sn-hju7en7k.googlevideo.com", "74.125.167.74");
        m.insert("rr1---sn-hju7en7r.googlevideo.com", "74.125.167.87");
        m.insert("play.google.com", "142.250.184.238");
        m.insert("rr3---sn-vh5ouxa-hjuz.googlevideo.com", "134.0.218.206");
        m.insert("rr3---sn-hju7enel.googlevideo.com", "74.125.98.40");
        m.insert("download.visualstudio.microsoft.com", "68.232.34.200");
        m.insert("i.ytimg.com", "142.250.186.150");
        m.insert("rr2---sn-hju7enel.googlevideo.com", "74.125.98.39");
        m.insert("rr2---sn-hju7en7k.googlevideo.com", "74.125.167.72");
        m.insert("rr3---sn-4g5lznl6.googlevideo.com", "74.125.173.40");
        m.insert("rr1---sn-hju7enll.googlevideo.com", "74.125.98.6");
        m.insert("rr6---sn-hju7en7r.googlevideo.com", "74.125.167.92");
        m.insert("www.gstatic.com", "142.250.185.99");
        m.insert("apis.google.com", "172.217.23.110");
        m.insert("adservice.google.com", "202.61.195.218");
        m.insert("mail.google.com", "142.250.186.37");
        m.insert("accounts.google.com", "172.217.16.205");
        m.insert("lh3.googleusercontent.com", "193.26.157.66");
        m.insert("accounts.youtube.com", "172.217.16.206");
        m.insert("ssl.gstatic.com", "142.250.184.195");
        m.insert("rr4---sn-hju7enll.googlevideo.com", "74.125.98.9");
        m.insert("rr2---sn-hju7enll.googlevideo.com", "74.125.98.7");
        m.insert("rr1---sn-hju7enel.googlevideo.com", "74.125.98.38");
        m.insert("rr5---sn-vh5ouxa-hjuz.googlevideo.com", "134.0.218.208");
        m.insert("i1.ytimg.com", "172.217.18.14");
        m.insert("plos.org", "162.159.135.42");
        m.insert("fonts.googleapis.com", "89.58.57.45");
        m.insert("genweb.plos.org", "104.26.1.141");
        m.insert("static.ads-twitter.com", "146.75.120.157");
        m.insert("www.google-analytics.com", "142.250.185.174");
        m.insert("rr1---sn-vh5ouxa-hju6.googlevideo.com", "213.202.6.140");
        m.insert("rr5---sn-vh5ouxa-hju6.googlevideo.com", "213.202.6.144");
        m.insert("rr5---sn-nv47zn7y.googlevideo.com", "173.194.15.74");
        m.insert("safebrowsing.googleapis.com", "202.61.195.218");
        m.insert("rr4---sn-vh5ouxa-hju6.googlevideo.com", "213.202.6.143");
        m.insert("rr4---sn-hju7en7r.googlevideo.com", "74.125.167.90");
        m.insert("r1---sn-hju7enel.googlevideo.com", "74.125.98.38");
        m.insert("rr1---sn-nv47zn7r.googlevideo.com", "173.194.15.38");
        m.insert("rr2---sn-vh5ouxa-hjuz.googlevideo.com", "134.0.218.205");
        m.insert("rr4---sn-nv47zn7r.googlevideo.com", "173.194.15.41");
        m.insert("www.google.com", "142.250.186.36");
        m.insert("youtube.com", "216.239.38.120");
        m.insert("youtu.be", "216.239.38.120");
        m.insert("www.youtube.com", "216.239.38.120");
        m.insert("yt3.ggpht.com", "142.250.186.36");
        m
    })
}

/// Resolve a host through the static offline-DNS map. Used only for bootstrap
/// hosts whose handshake must reach a known clean IP (DoH servers, etc.).
/// Discord domains are deliberately NOT mapped here — neither the apex nor any
/// subdomain: they must resolve via DoH so the real Discord anycast IPs
/// (162.159.x.x2x) land in the ping pool and are routed directly with
/// ClientHello fragmentation. The old suffix rules pointed every *.discord.*
/// subdomain at the ISP's SNI-forwarding host 203.32.120.226, but Cloudflare
/// Spectrum rejects that path with 1034 "Edge IP Restricted", so it only
/// poisoned the pool; keeping the apex out but routing subdomains there was
/// also inconsistent. Removing the suffix rules makes apex and subdomains
/// behave the same.
pub fn resolve_offline_dns(host: &str) -> Option<&'static str> {
    get_offline_dns().get(host).copied()
}
