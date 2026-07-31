use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Debug, Clone)]
pub struct Config {
    pub listen_port: u16,
    pub num_fragment: usize,
    pub fragment_sleep_ms: u64,
    pub log_every_sec: u64,
    pub allow_insecure: bool,
    pub socket_timeout_sec: u64,
    pub voice_socket_timeout_sec: u64,
    pub doh_max_retries: usize,
    pub doh_max_fails_before_switch: u32,
    pub doh_blacklist_sec: u64,
    pub doh_timeout_sec: u64,
    pub discord_ping_interval_sec: u64,
    pub discord_ping_timeout_sec: u64,
    pub discord_max_ips: usize,
    pub discord_min_rtt_ms: f64,
    pub data_dir: PathBuf,
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
            doh_max_retries: 5,
            doh_max_fails_before_switch: 3,
            doh_blacklist_sec: 300,
            doh_timeout_sec: 10,
            discord_ping_interval_sec: 30,
            discord_ping_timeout_sec: 3,
            discord_max_ips: 20,
            discord_min_rtt_ms: 1.0,
            data_dir: PathBuf::from("."),
        }
    }
}

pub static DOH_SERVERS: &[&str] = &[
    "https://cloudflare-dns.com/dns-query?dns=",
    "https://dns.google/dns-query?dns=",
    "https://doh.opendns.com/dns-query?dns=",
    "https://dns.quad9.net/dns-query?dns=",
    "https://doh.libredns.gr/dns-query?dns=",
    "https://dns.bitdefender.net/dns-query?dns=",
    "https://secure.avastdns.com/dns-query?dns=",
    "https://doh.cleanbrowsing.org/doh/dns-query?dns=",
    "https://doh.dns.sb/doh/dns-query?dns=",
    "https://doh.tiar.app/dns-query?dns=",
    "https://doh.dnswarden.com/dns-query?dns=",
    "https://doh.powerdns.org/dns-query?dns=",
    "https://dns.electrotm.org/dns-query?dns=",
    "https://cluster-1.gac.edu/dns-query?dns=",
    "https://dns.hostux.net/dns-query?dns=",
    "https://doh.securedns.eu/dns-query?dns=",
    "https://doh.ffmuc.net/dns-query?dns=",
    "https://dns.cmrg.net/dns-query?dns=",
    "https://doh.centraleu.pi-dns.com/dns-query?dns=",
    "https://doh.dns.live/dns-query?dns=",
    "https://dns.friendi.ca/dns-query?dns=",
    "https://doh.bortzmeyer.org/dns-query?dns=",
    "https://doh.airdns.org/dns-query?dns=",
    "https://dns.hyperpipe.surge.sh/dns-query?dns=",
    "https://dns.digitale-gesellschaft.ch/dns-query?dns=",
    "https://doh.ibk.pl/dns-query?dns=",
    "https://dns.rubyfish.io/dns-query?dns=",
    "https://doh.otk.ee/dns-query?dns=",
    "https://dns.joatmalatesta.net/dns-query?dns=",
    "https://doh.shecan.ir/dns-query?dns=",
    "https://1.1.1.1/dns-query?dns=",
    "https://1.0.0.1/dns-query?dns=",
    "https://8.8.8.8/dns-query?dns=",
    "https://8.8.4.4/dns-query?dns=",
    "https://9.9.9.9/dns-query?dns=",
    "https://149.112.112.112/dns-query?dns=",
    "https://208.67.222.222/dns-query?dns=",
    "https://208.67.220.220/dns-query?dns=",
    "https://185.228.168.9/dns-query?dns=",
    "https://185.228.169.9/dns-query?dns=",
    "https://76.76.19.19/dns-query?dns=",
    "https://76.223.122.150/dns-query?dns=",
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
