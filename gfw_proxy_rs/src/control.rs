use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::timeout;

use crate::config::DOH_SERVERS;
use crate::discord::DiscordManager;
use crate::doh::DohClient;
use crate::stats::{now_iso, StatsManager};
use crate::tun::TunManager;

/// Localhost-only control HTTP server for the Electron main process:
///   POST /scan   -> trigger an immediate DoH probe/rescan
///   GET  /status -> current DoH + probe ranking + connection stats
///
/// Every request must carry `Authorization: Bearer <token>` (or the `token`
/// query param), where `<token>` matches the `VEGORD_PROXY_CONTROL_TOKEN` value
/// passed at spawn time. Even on localhost this closes the attack surface:
/// without it any local process (or a malicious web page via DNS rebinding)
/// could probe the control API. When no token was configured the server refuses
/// all requests except a plain GET /status without auth, which is needed for
/// the Electron startup health check (it does not know the token yet at boot).
pub async fn run_control_server(
    doh: Arc<DohClient>,
    discord: Arc<DiscordManager>,
    stats: Arc<StatsManager>,
    tun: Arc<TunManager>,
    port: u16,
    token: &str,
) {
    let bind = format!("127.0.0.1:{}", port);
    let Ok(listener) = TcpListener::bind(&bind).await else {
        eprintln!("[{}] [CTRL] failed to bind {}: {}", now_iso(), bind, std::io::Error::last_os_error());
        return;
    };
    println!("[{}] [CTRL] control server listening on {}", now_iso(), bind);

    let token = token.to_string();
    loop {
        let Ok((mut socket, _)) = listener.accept().await else {
            continue;
        };
        let doh = Arc::clone(&doh);
        let discord = Arc::clone(&discord);
        let stats = Arc::clone(&stats);
        let tun = Arc::clone(&tun);
        let token = token.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 2048];
            let n = match timeout(Duration::from_secs(5), socket.read(&mut buf)).await {
                Ok(Ok(n)) if n > 0 => n,
                _ => return,
            };
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let first = req.lines().next().unwrap_or("");
            let mut parts = first.split_whitespace();
            let method = parts.next().unwrap_or("");
            let path_full = parts.next().unwrap_or("/");
            let path = path_full.split('?').next().unwrap_or("/");

            // Extract the bearer token / query token for auth.
            let auth = req
                .lines()
                .find(|l| l.to_ascii_lowercase().starts_with("authorization:"))
                .and_then(|l| l.split_once(':'))
                .map(|(_, v)| v.trim().trim_start_matches("Bearer ").trim().to_string())
                .unwrap_or_default();
            let query_token = path_full
                .split('?')
                .nth(1)
                .unwrap_or("")
                .split('&')
                .find_map(|kv| kv.strip_prefix("token="))
                .unwrap_or("")
                .to_string();
            let supplied = if !auth.is_empty() { auth } else { query_token };

            let authed = token.is_empty() || (supplied == token);

            let body = match (method, path, authed) {
                ("POST", "/scan", true) => {
                    doh.trigger_rescan("control-api");
                    r#"{"ok":true}"#.to_string()
                }
                ("GET", "/status", true) => status_json(&doh, &discord, &stats, &tun),
                ("GET", "/status", false) => {
                    let _ = socket
                        .write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .await;
                    return;
                }
                _ => {
                    let _ = socket
                        .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .await;
                    return;
                }
            };

            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = socket.write_all(resp.as_bytes()).await;
        });
    }
}

fn status_json(
    doh: &Arc<DohClient>,
    discord: &Arc<DiscordManager>,
    stats: &Arc<StatsManager>,
    tun: &Arc<TunManager>,
) -> String {
    use serde_json::json;

    let idx = doh.current_doh_index.load(std::sync::atomic::Ordering::Relaxed) % DOH_SERVERS.len();
    let best_ip = discord.get_best_ip().map(|ip| ip.to_string());
    let best_rtt = discord.get_best_rtt();

    let probes: Vec<serde_json::Value> = doh
        .probe_results
        .read()
        .iter()
        .map(|r| {
            json!({
                "index": r.index,
                "url": r.url,
                "avgRttMs": r.avg_rtt_ms,
                "successes": r.successes,
                "failures": r.failures
            })
        })
        .collect();

    // Per-IP Discord RTT + packet loss (feeds the internet-quality log).
    let discord_ips: Vec<serde_json::Value> = discord
        .get_ip_scores()
        .iter()
        .map(|(ip, rtt, loss)| {
            json!({
                "ip": ip.to_string(),
                "rttMs": rtt,
                "lossPct": loss
            })
        })
        .collect();

    let (conn_total, conn_ok, conn_filtered) = stats.conn_counts();
    let (ul_bytes, dl_bytes) = stats.traffic_totals();

    json!({
        "ok": true,
        "currentDohIndex": idx,
        "currentDoh": DOH_SERVERS[idx],
        "probeResults": probes,
        "totalSwitches": doh.total_switch_count.load(std::sync::atomic::Ordering::Relaxed),
        "discordBestIp": best_ip,
        "discordBestRtt": best_rtt,
        "connections": {
            "total": conn_total,
            "ok": conn_ok,
            "filtered": conn_filtered
        },
        "queries": {
            "total": doh.total_queries.load(std::sync::atomic::Ordering::Relaxed),
            "ok": doh.successful_queries.load(std::sync::atomic::Ordering::Relaxed),
            "fail": doh.failed_queries.load(std::sync::atomic::Ordering::Relaxed)
        },
        "traffic": {
            "ulBytes": ul_bytes,
            "dlBytes": dl_bytes
        },
        "tun": {
            "enabled": tun.enabled,
            "running": tun.is_running(),
            "routes": tun.route_count()
        },
        "discordIps": discord_ips
    })
    .to_string()
}
