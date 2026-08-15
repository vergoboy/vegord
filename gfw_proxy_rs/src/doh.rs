use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::net::IpAddr;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::cmp::Ordering as CmpOrdering;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use parking_lot::RwLock;
use reqwest::Client;
use serde_json::json;
use trust_dns_proto::op::{Message, Query};
use trust_dns_proto::rr::{Name, RecordType};

use crate::config::{Config, DOH_SERVERS, resolve_offline_dns};
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

/// One result of a DoH probe pass: measured RTT for a single candidate server.
#[derive(Debug, Clone)]
pub struct DohProbeResult {
    pub index: usize,
    pub url: &'static str,
    pub avg_rtt_ms: Option<f64>,
    pub successes: usize,
    pub failures: usize,
}

/// Shared state for a single in-flight DoH lookup so concurrent queries for the
/// same host coalesce into one network request instead of hammering DoH servers.
struct PendingQuery {
    done: bool,
    result: Option<Vec<IpAddr>>,
}

pub struct DohClient {
    config: Config,
    http_client: Client,
    dns_cache: RwLock<HashMap<String, Vec<IpAddr>>>,
    pending: parking_lot::RwLock<HashMap<String, Arc<tokio::sync::Mutex<PendingQuery>>>>,
    discord_mgr: Arc<DiscordManager>,
    pub current_doh_index: AtomicUsize,
    pub total_queries: AtomicU64,
    pub successful_queries: AtomicU64,
    pub failed_queries: AtomicU64,
    pub total_switch_count: AtomicU64,
    pub doh_perf: RwLock<Vec<DohPerf>>,
    pub probe_results: RwLock<Vec<DohProbeResult>>,
    probe_inflight: AtomicBool,
    last_scan_ms: AtomicI64,
    gateway_open_ms: AtomicI64,
    gateway_close_ms: AtomicI64,
}

impl DohClient {
    pub fn new(config: Config, discord_mgr: Arc<DiscordManager>) -> Arc<Self> {
        // Route DoH requests through our own HTTP proxy (like the original Python project).
        // This way the DoH server connection also benefits from offline-DNS clean IPs
        // and TLS ClientHello fragmentation. Without this, DoH servers are resolved
        // via the polluted system resolver and blocked by SNI-based filtering.
        //
        // This is NOT a circular dependency: every hostname-based DoH server is covered
        // by get_offline_dns(), so when the proxy resolves the DoH server's hostname it
        // returns immediately without recursing into another DoH query.
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

        let start_doh_index = config.preferred_doh_index.unwrap_or(0);

        Arc::new(Self {
            config,
            http_client,
            dns_cache: RwLock::new(HashMap::new()),
            pending: parking_lot::RwLock::new(HashMap::new()),
            discord_mgr,
            current_doh_index: AtomicUsize::new(start_doh_index),
            total_queries: AtomicU64::new(0),
            successful_queries: AtomicU64::new(0),
            failed_queries: AtomicU64::new(0),
            total_switch_count: AtomicU64::new(0),
            doh_perf: RwLock::new(perf),
            probe_results: RwLock::new(Vec::new()),
            probe_inflight: AtomicBool::new(false),
            last_scan_ms: AtomicI64::new(0),
            gateway_open_ms: AtomicI64::new(0),
            gateway_close_ms: AtomicI64::new(0),
        })
    }

    pub fn get_current_url(&self) -> &'static str {
        let idx = self.current_doh_index.load(Ordering::Relaxed) % DOH_SERVERS.len();
        DOH_SERVERS[idx]
    }

    /// True when this hostname is a Discord gateway endpoint. Used to detect
    /// Discord reconnects: a gateway connection opening shortly after one closed
    /// means Discord left the "connected" state and is reconnecting/loading.
    pub fn is_gateway_host(host: &str) -> bool {
        let h = host.to_lowercase();
        h.contains("gateway") && DiscordManager::is_discord_domain(&h)
    }

    pub fn note_gateway_connect(self: &Arc<Self>) {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let prev_open = self.gateway_open_ms.swap(now_ms, Ordering::Relaxed);
        let close_ms = self.gateway_close_ms.load(Ordering::Relaxed);

        if prev_open > 0 && close_ms > prev_open {
            let gap_ms = now_ms - close_ms;
            let window_ms = (self.config.doh_reconnect_window_sec as i64) * 1000;
            if gap_ms > 1000 && gap_ms < window_ms {
                self.trigger_rescan("gateway-reconnect");
            }
        }
    }

    pub fn note_gateway_disconnect(&self) {
        self.gateway_close_ms
            .store(chrono::Utc::now().timestamp_millis(), Ordering::Relaxed);
    }

    /// Fire-and-forget rescan, throttled by `doh_min_rescan_interval_sec`.
    pub fn trigger_rescan(self: &Arc<Self>, reason: &str) {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let last = self.last_scan_ms.load(Ordering::Relaxed);
        let min_gap = (self.config.doh_min_rescan_interval_sec as i64) * 1000;
        if now_ms - last < min_gap {
            return;
        }
        println!(
            "[{}] [DoH PROBE] rescan triggered (reason={})",
            now_iso(),
            reason
        );
        let this = Arc::clone(self);
        tokio::spawn(async move {
            this.probe_and_select().await;
        });
    }

    /// Probe every candidate DoH server, rank them by measured RTT and select the
    /// best (lowest avg RTT) as the active server. Parallelized with a concurrency
    /// cap so slow/unreachable servers do not stall the whole pass.
    pub async fn probe_and_select(self: &Arc<Self>) -> Vec<DohProbeResult> {
        if self.probe_inflight.swap(true, Ordering::SeqCst) {
            return self.probe_results.read().clone();
        }
        let now_ms = chrono::Utc::now().timestamp_millis();
        self.last_scan_ms.store(now_ms, Ordering::Relaxed);

        let attempts = self.config.doh_probe_attempts.max(1);
        let timeout_ms = (self.config.doh_probe_timeout_sec.max(1) as f64 * 1000.0) as u64;
        let concurrency = self.config.doh_probe_concurrency.max(1);

        let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrency));
        let mut tasks = Vec::with_capacity(DOH_SERVERS.len());
        for (i, url) in DOH_SERVERS.iter().enumerate() {
            let sem = Arc::clone(&semaphore);
            let this = Arc::clone(self);
            let url = *url;
            tasks.push(tokio::spawn(async move {
                let _permit = sem.acquire().await;
                let mut rtts = Vec::new();
                let mut failures = 0usize;
                for _ in 0..attempts {
                    match this.probe_server(url, timeout_ms).await {
                        Some(rtt) => rtts.push(rtt),
                        None => failures += 1,
                    }
                }
                let avg = if rtts.is_empty() {
                    None
                } else {
                    Some(rtts.iter().sum::<f64>() / rtts.len() as f64)
                };
                DohProbeResult {
                    index: i,
                    url,
                    avg_rtt_ms: avg,
                    successes: rtts.len(),
                    failures,
                }
            }));
        }

        let mut results = Vec::with_capacity(DOH_SERVERS.len());
        for task in tasks {
            if let Ok(r) = task.await {
                results.push(r);
            }
        }

        results.sort_by(|a, b| match (a.avg_rtt_ms, b.avg_rtt_ms) {
            (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(CmpOrdering::Equal),
            (Some(_), None) => CmpOrdering::Less,
            (None, Some(_)) => CmpOrdering::Greater,
            (None, None) => CmpOrdering::Equal,
        });

        if let Some(best) = results.first() {
            if best.avg_rtt_ms.is_some() {
                let idx = best.index;
                // Keep the current server if it is close enough to the best one.
                // RTTs on a filtered line are noisy (~5ms apart); flipping the
                // active server on every rescan just resets per-server stats and
                // churns the DNS cache, so only switch on a meaningful gain.
                let curr_idx = self.current_doh_index.load(Ordering::Relaxed) % DOH_SERVERS.len();
                let keep = results.iter().find(|r| r.index == curr_idx);
                let mut switched = idx != curr_idx;
                if let Some(c) = keep {
                    if let (Some(cur_rtt), Some(best_rtt)) = (c.avg_rtt_ms, best.avg_rtt_ms) {
                        let margin = self.config.doh_switch_margin_ms;
                        if cur_rtt <= best_rtt + margin {
                            switched = false;
                            println!(
                                "[{}] [DoH PROBE] keeping current #{} {} (RTT={:.1}ms, best={:.1}ms)",
                                now_iso(),
                                curr_idx,
                                DOH_SERVERS[curr_idx],
                                cur_rtt,
                                best_rtt
                            );
                        }
                    }
                }

                if switched {
                    self.current_doh_index.store(idx, Ordering::Relaxed);
                    {
                        let mut perf = self.doh_perf.write();
                        perf[idx].blacklisted_until = 0;
                    }
                    println!(
                        "[{}] [DoH PROBE] selected #{} {} (avg RTT={:.1}ms)",
                        now_iso(),
                        idx,
                        DOH_SERVERS[idx],
                        best.avg_rtt_ms.unwrap_or(0.0)
                    );
                }
            }
        }

        self.probe_results.write().clone_from(&results);
        self.probe_inflight.store(false, Ordering::SeqCst);

        let mut lines = vec![format!("DoH Probe Ranking @ {}", now_iso())];
        for (rank, r) in results.iter().enumerate() {
            let rtt = r
                .avg_rtt_ms
                .map_or("FAIL".to_string(), |v| format!("{:.1}ms", v));
            lines.push(format!(
                "  #{:2} RANK={:2} RTT={:>9} OK={} FAIL={} {}",
                r.index, rank + 1, rtt, r.successes, r.failures, r.url
            ));
        }
        if let Ok(mut f) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.config.data_dir.join("DoH_probe_ranking.txt"))
        {
            let _ = writeln!(f, "{}", lines.join("\n"));
        }

        results
    }

    /// One DoH measurement: resolve `discord.com` through the given server and
    /// return the round-trip time in ms on success, or None on timeout/failure.
    async fn probe_server(&self, url: &str, timeout_ms: u64) -> Option<f64> {
        let query_bytes = build_doh_query("discord.com")?;
        let b64_q = URL_SAFE_NO_PAD.encode(&query_bytes);
        let full_url = format!("{}{}", url, b64_q);
        let t0 = Instant::now();
        let fut = self
            .http_client
            .get(&full_url)
            .query(&[("type", "A"), ("ct", "application/dns-message")])
            .header("accept", "application/dns-message")
            .send();
        match tokio::time::timeout(Duration::from_millis(timeout_ms), fut).await {
            Ok(Ok(resp)) if resp.status().is_success() => {
                let bytes = resp.bytes().await.ok()?;
                let msg = Message::from_vec(&bytes).ok()?;
                let has_a = msg.answers().iter().any(|r| r.record_type() == RecordType::A);
                if has_a {
                    Some((t0.elapsed().as_secs_f64() * 1000.0 * 10.0).round() / 10.0)
                } else {
                    None
                }
            }
            _ => None,
        }
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
        self.query_all(server_name)
            .await
            .and_then(|ips| ips.first().copied())
    }

    /// Resolve a host to all of its A records (via static mapping, the DNS
    /// cache, or DoH). The full list is kept so the proxy can fall back to an
    /// alternate IP when the GFW resets connections to the primary one.
    pub async fn query_all(&self, server_name: &str) -> Option<Vec<IpAddr>> {
        // 1. Check offline DNS static mapping
        if let Some(ip_str) = resolve_offline_dns(server_name) {
            if let Ok(ip) = IpAddr::from_str(ip_str) {
                println!("[{}] [DNS] offline {} -> {}", now_iso(), server_name, ip);
                return Some(vec![ip]);
            }
        }

        // 2. Check in-memory DNS cache
        {
            let cache = self.dns_cache.read();
            if let Some(ips) = cache.get(server_name) {
                println!("[{}] [DNS] cached {} -> {:?}", now_iso(), server_name, ips);
                return Some(ips.clone());
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
            return guard.result.clone();
        }

        // We are the leader: perform the actual DoH resolution while followers wait.
        let result = self.resolve_via_doh(server_name).await;
        guard.done = true;
        guard.result = result.clone();
        drop(guard);

        // Let a future query for this host start fresh (the DNS cache absorbs hits).
        self.pending.write().remove(server_name);
        result
    }

    async fn resolve_via_doh(&self, server_name: &str) -> Option<Vec<IpAddr>> {
        self.total_queries.fetch_add(1, Ordering::Relaxed);
        let is_discord = DiscordManager::is_discord_domain(server_name);
        println!(
            "[{}] [DNS] resolving {} via DoH{}",
            now_iso(),
            server_name,
            if is_discord { " [discord]" } else { "" }
        );

        let mut fail_count = 0;
        let mut fail_kind = "";
        let max_tries = std::cmp::min(DOH_SERVERS.len(), self.config.doh_max_retries);

        for _attempt in 0..max_tries {
            let idx = self.current_doh_index.load(Ordering::Relaxed) % DOH_SERVERS.len();
            let doh_base = DOH_SERVERS[idx];

            if let Some(query_bytes) = build_doh_query(server_name) {
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
                                let mut all_ips = Vec::new();

                                for record in msg.answers() {
                                    if record.record_type() == RecordType::A {
                                        if let Some(rdata) = record.data() {
                                            if let Some(a_rec) = rdata.as_a() {
                                                let ip = IpAddr::V4(a_rec.0);
                                                if !all_ips.contains(&ip) {
                                                    all_ips.push(ip);
                                                }
                                            }
                                        }
                                    }
                                }

                                if !all_ips.is_empty() {
                                    self.dns_cache
                                        .write()
                                        .insert(server_name.to_string(), all_ips.clone());
                                    if is_discord {
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
                                        "[{}] [DNS] {} -> {:?} (RTT={}ms)",
                                        now_iso(),
                                        server_name,
                                        all_ips,
                                        rtt
                                    );
                                    return Some(all_ips);
                                } else {
                                    println!(
                                        "[{}] [DoH WARN] No A record from server #{} for {}",
                                        now_iso(),
                                        idx,
                                        server_name
                                    );
                                    fail_kind = "no_a";
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
                        fail_kind = "http";
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
                        fail_kind = "error";
                    }
                }
            }

            let is_critical = DiscordManager::is_discord_domain(server_name)
                || server_name.ends_with("vergoboy.ir");
            // A valid DNS answer with no A record is the resolver answering
            // correctly. For domains guaranteed to exist (Discord, the panel)
            // an empty answer is a resolver-side problem; for asset subdomains
            // (e.g. badges.vegord.dev) it is a domain issue — advance to the
            // next resolver without blacklisting, so a healthy resolver is not
            // poisoned for 300s over one unknown domain.
            let resolver_broken = fail_kind == "error"
                || fail_kind == "http"
                || (fail_kind == "no_a" && is_critical);
            if resolver_broken {
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
            } else if fail_kind == "no_a" {
                self.switch_doh();
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
            .flat_map(|(k, ips)| ips.iter().map(move |ip| (k.clone(), *ip)))
            .collect();
        list.sort_by(|a, b| a.0.cmp(&b.0));
        list
    }
}

fn build_doh_query(server_name: &str) -> Option<Vec<u8>> {
    let name = Name::from_str(server_name).ok()?;
    let mut msg = Message::new();
    let query = Query::query(name, RecordType::A);
    msg.add_query(query);
    msg.set_recursion_desired(true);
    msg.to_vec().ok()
}
