use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::net::IpAddr;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use parking_lot::RwLock;
use reqwest::Client;
use serde_json::json;
use trust_dns_proto::op::{Message, Query};
use trust_dns_proto::rr::{Name, RecordType};

use crate::config::{Config, DOH_SERVERS, get_offline_dns};
use crate::discord::DiscordManager;
use crate::stats::now_iso;

#[derive(Debug, Clone)]
pub struct DohPerf {
    pub last_rtt: Option<f64>,
    pub fail_count: u32,
    pub success_count: u32,
    pub blacklisted_until: u64,
    pub rtt_ms: Vec<f64>,
}

/// Shared state for a single in-flight DoH lookup so concurrent queries for the
/// same host coalesce into one network request instead of hammering DoH servers.
struct PendingQuery {
    done: bool,
    result: Option<IpAddr>,
}

pub struct DohClient {
    config: Config,
    http_client: Client,
    dns_cache: RwLock<HashMap<String, IpAddr>>,
    pending: parking_lot::RwLock<HashMap<String, Arc<tokio::sync::Mutex<PendingQuery>>>>,
    discord_mgr: Arc<DiscordManager>,
    pub current_doh_index: AtomicUsize,
    pub total_queries: AtomicU64,
    pub successful_queries: AtomicU64,
    pub failed_queries: AtomicU64,
    pub total_switch_count: AtomicU64,
    pub doh_perf: RwLock<Vec<DohPerf>>,
}

impl DohClient {
    pub fn new(config: Config, discord_mgr: Arc<DiscordManager>) -> Arc<Self> {
        // Route DoH requests through our own HTTP proxy (like the original Python project).
        // This way the DoH server connection also benefits from offline-DNS clean IPs
        // and TLS ClientHello fragmentation. Without this, DoH servers are resolved
        // via the polluted system resolver and blocked by SNI-based filtering.
        let proxy_url = format!("http://127.0.0.1:{}", config.listen_port);
        let http_client = Client::builder()
            .timeout(Duration::from_secs(config.doh_timeout_sec))
            .danger_accept_invalid_certs(config.allow_insecure)
            .proxy(reqwest::Proxy::all(&proxy_url).unwrap_or_else(|_| reqwest::Proxy::all("http://127.0.0.1:4500").unwrap()))
            .build()
            .unwrap_or_else(|_| Client::new());

        let perf = vec![
            DohPerf {
                last_rtt: None,
                fail_count: 0,
                success_count: 0,
                blacklisted_until: 0,
                rtt_ms: Vec::new(),
            };
            DOH_SERVERS.len()
        ];

        Arc::new(Self {
            config,
            http_client,
            dns_cache: RwLock::new(HashMap::new()),
            pending: parking_lot::RwLock::new(HashMap::new()),
            discord_mgr,
            current_doh_index: AtomicUsize::new(0),
            total_queries: AtomicU64::new(0),
            successful_queries: AtomicU64::new(0),
            failed_queries: AtomicU64::new(0),
            total_switch_count: AtomicU64::new(0),
            doh_perf: RwLock::new(perf),
        })
    }

    pub fn get_current_url(&self) -> &'static str {
        let idx = self.current_doh_index.load(Ordering::Relaxed) % DOH_SERVERS.len();
        DOH_SERVERS[idx]
    }

    fn log_switch_event(&self, old_url: &str, new_url: &str, total_switches: u64) {
        let event = json!({
            "time": now_iso(),
            "from": old_url,
            "to": new_url,
            "total_switches": total_switches
        });
        let log_path: PathBuf = self.config.data_dir.join("DoH_switch_log.txt");
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(log_path) {
            let _ = writeln!(f, "{}", event);
        }
    }

    pub fn switch_doh(&self) -> &'static str {
        let old_idx = self.current_doh_index.load(Ordering::Relaxed) % DOH_SERVERS.len();
        let old_url = DOH_SERVERS[old_idx];
        let total_switches = self.total_switch_count.fetch_add(1, Ordering::Relaxed) + 1;

        let now_sec = chrono::Utc::now().timestamp() as u64;
        let perf_guard = self.doh_perf.read();

        let mut next_idx = (old_idx + 1) % DOH_SERVERS.len();
        let mut tried = 0;
        while tried < DOH_SERVERS.len() {
            if perf_guard[next_idx].blacklisted_until <= now_sec {
                break;
            }
            next_idx = (next_idx + 1) % DOH_SERVERS.len();
            tried += 1;
        }

        self.current_doh_index.store(next_idx, Ordering::Relaxed);
        let new_url = DOH_SERVERS[next_idx];

        println!(
            "[{}] [DoH SWITCH] #{} {} -> {}",
            now_iso(),
            total_switches,
            old_url,
            new_url
        );
        self.log_switch_event(old_url, new_url, total_switches);
        new_url
    }

    pub fn blacklist_current(&self) {
        let idx = self.current_doh_index.load(Ordering::Relaxed) % DOH_SERVERS.len();
        let until = (chrono::Utc::now().timestamp() as u64) + self.config.doh_blacklist_sec;
        let mut perf_guard = self.doh_perf.write();
        perf_guard[idx].blacklisted_until = until;
        println!(
            "[{}] [DoH BLACKLIST] {} blacklisted for {}s",
            now_iso(),
            DOH_SERVERS[idx],
            self.config.doh_blacklist_sec
        );
    }

    pub async fn query(&self, server_name: &str) -> Option<IpAddr> {
        // 1. Check offline DNS static mapping
        if let Some(&ip_str) = get_offline_dns().get(server_name) {
            if let Ok(ip) = IpAddr::from_str(ip_str) {
                println!("[{}] [DNS] offline {} -> {}", now_iso(), server_name, ip);
                return Some(ip);
            }
        }

        // 2. Check in-memory DNS cache
        {
            let cache = self.dns_cache.read();
            if let Some(&ip) = cache.get(server_name) {
                println!("[{}] [DNS] cached {} -> {}", now_iso(), server_name, ip);
                return Some(ip);
            }
        }

        // 3. Single-flight: coalesce concurrent queries for the same host so the
        //    DoH servers are not hammered with parallel duplicate lookups.
        let waiter = {
            let mut pending = self.pending.write();
            match pending.get(server_name) {
                Some(shared) => {
                    println!(
                        "[{}] [DNS] coalescing in-flight query for {}",
                        now_iso(),
                        server_name
                    );
                    shared.clone()
                }
                None => {
                    let shared = Arc::new(tokio::sync::Mutex::new(PendingQuery {
                        done: false,
                        result: None,
                    }));
                    pending.insert(server_name.to_string(), shared.clone());
                    shared
                }
            }
        };

        let mut guard = waiter.lock().await;
        if guard.done {
            return guard.result;
        }

        // We are the leader: perform the actual DoH resolution while followers wait.
        let result = self.resolve_via_doh(server_name).await;
        guard.done = true;
        guard.result = result;
        drop(guard);

        // Let a future query for this host start fresh (the DNS cache absorbs hits).
        self.pending.write().remove(server_name);
        result
    }

    async fn resolve_via_doh(&self, server_name: &str) -> Option<IpAddr> {
        self.total_queries.fetch_add(1, Ordering::Relaxed);
        let is_discord = DiscordManager::is_discord_domain(server_name);
        println!(
            "[{}] [DNS] resolving {} via DoH{}",
            now_iso(),
            server_name,
            if is_discord { " [discord]" } else { "" }
        );

        let mut fail_count = 0;
        let max_tries = std::cmp::min(DOH_SERVERS.len(), self.config.doh_max_retries);

        for _attempt in 0..max_tries {
            let idx = self.current_doh_index.load(Ordering::Relaxed) % DOH_SERVERS.len();
            let doh_base = DOH_SERVERS[idx];

            let query_res = (|| {
                let name = Name::from_str(server_name).ok()?;
                let mut msg = Message::new();
                let query = Query::query(name, RecordType::A);
                msg.add_query(query);
                msg.set_recursion_desired(true);
                msg.to_vec().ok()
            })();

            if let Some(query_bytes) = query_res {
                let b64_q = URL_SAFE_NO_PAD.encode(&query_bytes);
                let full_url = format!("{}{}", doh_base, b64_q);
                let t0 = Instant::now();

                let req_fut = self
                    .http_client
                    .get(&full_url)
                    .query(&[("type", "A"), ("ct", "application/dns-message")])
                    .header("accept", "application/dns-message")
                    .send();

                match req_fut.await {
                    Ok(resp) if resp.status().is_success() => {
                        let rtt = (t0.elapsed().as_secs_f64() * 1000.0 * 10.0).round() / 10.0;
                        if let Ok(bytes) = resp.bytes().await {
                            if let Ok(msg) = Message::from_vec(&bytes) {
                                let mut resolved_first: Option<IpAddr> = None;
                                let mut all_ips = Vec::new();

                                for record in msg.answers() {
                                    if record.record_type() == RecordType::A {
                                        if let Some(rdata) = record.data() {
                                            if let Some(a_rec) = rdata.as_a() {
                                                let ip = IpAddr::V4(a_rec.0);
                                                if resolved_first.is_none() {
                                                    resolved_first = Some(ip);
                                                }
                                                all_ips.push(ip);
                                            }
                                        }
                                    }
                                }

                                if let Some(first_ip) = resolved_first {
                                    self.dns_cache.write().insert(server_name.to_string(), first_ip);
                                    if is_discord && !all_ips.is_empty() {
                                        self.discord_mgr.feed_ips(&all_ips, self.config.discord_max_ips);
                                    }

                                    self.successful_queries.fetch_add(1, Ordering::Relaxed);
                                    let mut perf = self.doh_perf.write();
                                    perf[idx].success_count += 1;
                                    perf[idx].last_rtt = Some(rtt);
                                    perf[idx].rtt_ms.push(rtt);
                                    if perf[idx].rtt_ms.len() > 50 {
                                        perf[idx].rtt_ms.remove(0);
                                    }

                                    println!(
                                        "[{}] [DNS] {} -> {} (RTT={}ms)",
                                        now_iso(),
                                        server_name,
                                        first_ip,
                                        rtt
                                    );
                                    return Some(first_ip);
                                } else {
                                    println!(
                                        "[{}] [DoH WARN] No A record from server #{} for {}",
                                        now_iso(),
                                        idx,
                                        server_name
                                    );
                                }
                            }
                        }
                    }
                    Ok(resp) => {
                        println!(
                            "[{}] [DoH WARN] Server #{} returned HTTP {} for {}",
                            now_iso(),
                            idx,
                            resp.status(),
                            server_name
                        );
                    }
                    Err(err) => {
                        if err.is_timeout() {
                            println!(
                                "[{}] [DoH TIMEOUT] Server #{} timeout ({}s)",
                                now_iso(),
                                idx,
                                self.config.doh_timeout_sec
                            );
                        } else {
                            println!("[{}] [DoH ERR] Server #{}: {}", now_iso(), idx, err);
                        }
                    }
                }
            }

            {
                let mut perf = self.doh_perf.write();
                perf[idx].fail_count += 1;
            }
            fail_count += 1;
            if fail_count >= self.config.doh_max_fails_before_switch {
                self.blacklist_current();
                self.switch_doh();
                fail_count = 0;
            }
        }

        self.failed_queries.fetch_add(1, Ordering::Relaxed);
        println!(
            "[{}] [DNS FAIL] All DoH servers failed for {}",
            now_iso(),
            server_name
        );
        None
    }

    pub fn get_cache_snapshot(&self) -> Vec<(String, IpAddr)> {
        let cache = self.dns_cache.read();
        let mut list: Vec<(String, IpAddr)> = cache
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        list.sort_by(|a, b| a.0.cmp(&b.0));
        list
    }
}
