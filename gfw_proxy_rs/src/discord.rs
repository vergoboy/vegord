use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use parking_lot::RwLock;
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::config::{Config, DISCORD_DOMAINS};

#[derive(Debug, Clone)]
pub struct DiscordIpInfo {
    pub rtt: Option<f64>,
    pub last_ping: u64,
    pub samples: Vec<f64>,
    pub implausible_logged: bool,
    // Loss is measured separately from TCP RTT (spec section 5.2/4.3): the UDP
    // voice relay feeds per-heartbeat miss ratios via note_loss_sample, and the
    // combined score ranks a high-loss route below a slightly slower one.
    pub loss_pct: Option<f64>,
}

// A high-loss route degrades voice quality far more than a few ms of extra RTT,
// so 1% loss is penalized as if it added 10ms to the route.
const LOSS_WEIGHT_MS: f64 = 10.0;

fn combined_score(rtt: Option<f64>, loss_pct: Option<f64>) -> f64 {
    match rtt {
        None => f64::MAX,
        Some(r) => r + loss_pct.unwrap_or(0.0) * LOSS_WEIGHT_MS,
    }
}

pub struct DiscordManager {
    ips: RwLock<HashMap<IpAddr, DiscordIpInfo>>,
    best_ip: RwLock<Option<IpAddr>>,
    best_rtt: RwLock<Option<f64>>,
}

impl DiscordManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            ips: RwLock::new(HashMap::new()),
            best_ip: RwLock::new(None),
            best_rtt: RwLock::new(None),
        })
    }

    pub fn is_discord_domain(host: &str) -> bool {
        let h = host.to_lowercase();
        if DISCORD_DOMAINS.iter().any(|d| h.contains(d)) {
            return true;
        }
        h.ends_with(".discord.com")
            || h.ends_with(".discord.gg")
            || h.ends_with(".discordapp.com")
            || h.ends_with(".discordapp.net")
            || h.ends_with(".discord.media")
    }

    pub fn feed_ips(&self, new_ips: &[IpAddr], max_ips: usize) {
        let mut ips = self.ips.write();
        for &ip in new_ips {
            if ips.len() >= max_ips {
                break;
            }
            if !ips.contains_key(&ip) {
                ips.insert(
                    ip,
                    DiscordIpInfo {
                        rtt: None,
                        last_ping: 0,
                        samples: Vec::new(),
                        implausible_logged: false,
                        loss_pct: None,
                    },
                );
                println!("[{}] [DISCORD] added IP {} to ping pool", crate::stats::now_iso(), ip);
            }
        }
    }

    pub fn get_best_ip(&self) -> Option<IpAddr> {
        *self.best_ip.read()
    }

    pub fn get_best_rtt(&self) -> Option<f64> {
        *self.best_rtt.read()
    }

    pub fn get_ips_snapshot(&self) -> Vec<(IpAddr, Option<f64>, usize)> {
        let ips = self.ips.read();
        ips.iter()
            .map(|(&ip, info)| (ip, info.rtt, info.samples.len()))
            .collect()
    }

    /// (ip, rtt_ms, loss_pct) for every known IP — feeds the `discord_ip_scores`
    /// field of the local preset (spec section 4.3).
    pub fn get_ip_scores(&self) -> Vec<(IpAddr, Option<f64>, Option<f64>)> {
        let ips = self.ips.read();
        ips.iter()
            .map(|(&ip, info)| (ip, info.rtt, info.loss_pct))
            .collect()
    }

    /// Record a voice-path loss measurement (percent of missed heartbeats over
    /// the last window) for a Discord voice IP, fed by the UDP relay.
    pub fn note_loss_sample(&self, ip: IpAddr, loss_pct: f64) {
        let mut ips = self.ips.write();
        if let Some(info) = ips.get_mut(&ip) {
            info.loss_pct = Some(loss_pct);
        }
    }

    /// Return the best-ranked IP that is not `except`. Used by the voice
    /// failover: when the current voice route goes dead, the relay asks for the
    /// next-best candidate (by combined loss+RTT score) instead of guessing.
    pub fn next_best_ip(&self, except: Option<IpAddr>) -> Option<IpAddr> {
        let ips = self.ips.read();
        let mut ranked: Vec<(&IpAddr, f64)> = ips
            .iter()
            .map(|(ip, info)| (ip, combined_score(info.rtt, info.loss_pct)))
            .collect();
        ranked.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        ranked
            .into_iter()
            .find(|(ip, _)| Some(**ip) != except)
            .map(|(ip, _)| *ip)
    }

    pub fn start_pinger(self: Arc<Self>, config: Config) {
        tokio::spawn(async move {
            let interval_dur = Duration::from_secs(config.discord_ping_interval_sec);
            let ping_timeout = Duration::from_secs(config.discord_ping_timeout_sec);

            loop {
                tokio::time::sleep(interval_dur).await;

                let targets: Vec<IpAddr> = {
                    let ips = self.ips.read();
                    ips.keys().copied().collect()
                };

                if targets.is_empty() {
                    continue;
                }

                let mut best_score_val = f64::MAX;
                let mut best_rtt_val = f64::MAX;
                let mut best_ip_val: Option<IpAddr> = None;

                for ip in targets {
                    let addr = SocketAddr::new(ip, 443);
                    let t0 = Instant::now();

                    let res = timeout(ping_timeout, TcpStream::connect(addr)).await;
                    let rtt_opt = match res {
                        Ok(Ok(stream)) => {
                            let rtt = t0.elapsed().as_secs_f64() * 1000.0;
                            drop(stream);
                            let rtt = (rtt * 10.0).round() / 10.0;
                            if rtt < config.discord_min_rtt_ms {
                                None
                            } else {
                                Some(rtt)
                            }
                        }
                        _ => None,
                    };

                    let now_secs = chrono::Utc::now().timestamp() as u64;
                    let mut ips = self.ips.write();
                    if let Some(info) = ips.get_mut(&ip) {
                        info.last_ping = now_secs;
                        info.rtt = rtt_opt;
                        if let Some(rtt) = rtt_opt {
                            if info.implausible_logged {
                                info.implausible_logged = false;
                            }
                            info.samples.push(rtt);
                            if info.samples.len() > 10 {
                                info.samples.remove(0);
                            }
                            let avg: f64 = info.samples.iter().sum::<f64>() / info.samples.len() as f64;
                            // Rank by the combined score (RTT + loss penalty) so a
                            // low-latency but lossy route does not beat a clean one.
                            let score = combined_score(Some(avg), info.loss_pct);
                            if score < best_score_val {
                                best_score_val = score;
                                best_rtt_val = avg;
                                best_ip_val = Some(ip);
                            }
                        } else if !info.implausible_logged {
                            // An implausible (likely ISP-intercepted) RTT repeats
                            // every ping cycle for the same IP; log it once to
                            // avoid spamming the connection log with thousands
                            // of identical lines.
                            info.implausible_logged = true;
                            println!(
                                "[{}] [DISCORD] implausible RTT for {} (< {}ms, likely ISP interception), ignoring",
                                crate::stats::now_iso(),
                                ip,
                                config.discord_min_rtt_ms
                            );
                        }
                    }
                }

                if let Some(bip) = best_ip_val {
                    let rounded_best = (best_rtt_val * 10.0).round() / 10.0;
                    *self.best_ip.write() = Some(bip);
                    *self.best_rtt.write() = Some(rounded_best);
                    println!(
                        "[{}] [DISCORD] best IP updated: {} (avg RTT={}ms)",
                        crate::stats::now_iso(),
                        bip,
                        rounded_best
                    );
                }
            }
        });
    }
}
