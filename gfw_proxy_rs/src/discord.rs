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
                            if avg < best_rtt_val {
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
