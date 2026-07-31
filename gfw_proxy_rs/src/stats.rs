use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use parking_lot::RwLock;

use crate::config::{Config, DOH_SERVERS, get_offline_dns};
use crate::discord::DiscordManager;
use crate::doh::DohClient;

pub fn now_iso() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}

pub struct StatsManager {
    config: Config,
    doh: Arc<DohClient>,
    discord: Arc<DiscordManager>,
    pub conn_total: AtomicU64,
    pub conn_success: AtomicU64,
    pub conn_filtered: AtomicU64,
    ul_traffic: RwLock<HashMap<IpAddr, u64>>,
    dl_traffic: RwLock<HashMap<IpAddr, u64>>,
}

impl StatsManager {
    pub fn new(config: Config, doh: Arc<DohClient>, discord: Arc<DiscordManager>) -> Arc<Self> {
        Arc::new(Self {
            config,
            doh,
            discord,
            conn_total: AtomicU64::new(0),
            conn_success: AtomicU64::new(0),
            conn_filtered: AtomicU64::new(0),
            ul_traffic: RwLock::new(HashMap::new()),
            dl_traffic: RwLock::new(HashMap::new()),
        })
    }

    pub fn record_ul(&self, ip: IpAddr, bytes: u64) {
        let mut ul = self.ul_traffic.write();
        *ul.entry(ip).or_insert(0) += bytes;
    }

    pub fn record_dl(&self, ip: IpAddr, bytes: u64) {
        let mut dl = self.dl_traffic.write();
        *dl.entry(ip).or_insert(0) += bytes;
    }

    pub fn start_log_writer(self: Arc<Self>) {
        tokio::spawn(async move {
            let log_interval = Duration::from_secs(self.config.log_every_sec);

            loop {
                tokio::time::sleep(log_interval).await;

                let mut lines = Vec::new();
                lines.push("=== DoH Network Health ===".to_string());
                lines.push(format!("Uptime: {}", now_iso()));
                let doh_idx = self.doh.current_doh_index.load(Ordering::Relaxed) % DOH_SERVERS.len();
                lines.push(format!("Current DoH:  #{} {}", doh_idx, DOH_SERVERS[doh_idx]));
                lines.push(format!("Total Switches: {}", self.doh.total_switch_count.load(Ordering::Relaxed)));
                lines.push(format!(
                    "Queries: {} OK={} FAIL={}",
                    self.doh.total_queries.load(Ordering::Relaxed),
                    self.doh.successful_queries.load(Ordering::Relaxed),
                    self.doh.failed_queries.load(Ordering::Relaxed)
                ));
                lines.push(format!(
                    "Connections: {} OK={} FILTERED={}",
                    self.conn_total.load(Ordering::Relaxed),
                    self.conn_success.load(Ordering::Relaxed),
                    self.conn_filtered.load(Ordering::Relaxed)
                ));
                lines.push(String::new());

                lines.push("--- Discord Best IP ---".to_string());
                if let Some(bip) = self.discord.get_best_ip() {
                    let rtt_str = self.discord.get_best_rtt().map_or("N/A".to_string(), |r| format!("{}ms", r));
                    let pool = self.discord.get_ips_snapshot();
                    lines.push(format!("  Best: {} (avg RTT={})", bip, rtt_str));
                    lines.push(format!("  Pool ({}):", pool.len()));
                    for (dip, rtt_opt, samples) in pool {
                        let r = rtt_opt.map_or("N/A".to_string(), |val| format!("{}ms", val));
                        lines.push(format!("    {:>15}  RTT={:>7}  samples={}", dip, r, samples));
                    }
                } else {
                    lines.push("  (no Discord IPs discovered yet)".to_string());
                }
                lines.push(String::new());

                lines.push("--- DoH Server Performance (last RTT) ---".to_string());
                {
                    let perf_guard = self.doh.doh_perf.read();
                    for (i, p) in perf_guard.iter().enumerate() {
                        let rtt_str = p.last_rtt.map_or("N/A".to_string(), |r| format!("{}ms", r));
                        lines.push(format!(
                            "  #{:2} RTT={:>8} OK={:3} FAIL={:2} {}",
                            i, rtt_str, p.success_count, p.fail_count, DOH_SERVERS[i]
                        ));
                    }
                }
                lines.push(String::new());

                lines.push("--- DNS Cache ---".to_string());
                for (domain, ip) in self.doh.get_cache_snapshot() {
                    lines.push(format!("  {} -> {}", domain, ip));
                }
                lines.push(String::new());

                lines.push("--- Traffic Stats ---".to_string());
                let ul_map = self.ul_traffic.read();
                let dl_map = self.dl_traffic.read();

                let mut all_ips: Vec<IpAddr> = ul_map.keys().chain(dl_map.keys()).copied().collect();
                all_ips.sort();
                all_ips.dedup();

                let mut reverse_dns: HashMap<IpAddr, String> = HashMap::new();
                for (k, v) in get_offline_dns().iter() {
                    if let Ok(ip) = v.parse::<IpAddr>() {
                        reverse_dns.insert(ip, k.to_string());
                    }
                }
                for (domain, ip) in self.doh.get_cache_snapshot() {
                    reverse_dns.insert(ip, domain);
                }

                for ip in all_ips {
                    let up_kb = (ul_map.get(&ip).copied().unwrap_or(0) as f64) / 1024.0;
                    let down_kb = (dl_map.get(&ip).copied().unwrap_or(0) as f64) / 1024.0;
                    let host = reverse_dns.get(&ip).map(|s| s.as_str()).unwrap_or("?");
                    let filtered = if down_kb < 1.0 { "yes" } else { "---" };
                    lines.push(format!(
                        "  {}: UL={:.3}KB DL={:.3}KB filtered={} host={}",
                        ip, up_kb, down_kb, filtered, host
                    ));
                }

                let out_path: PathBuf = self.config.data_dir.join("DNS_IP_traffic_info.txt");
                if let Ok(mut f) = File::create(out_path) {
                    let _ = writeln!(f, "{}", lines.join("\n"));
                }
            }
        });
    }
}
