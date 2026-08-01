use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::timeout;

use crate::config::DOH_SERVERS;
use crate::discord::DiscordManager;
use crate::doh::DohClient;
use crate::stats::now_iso;

/// Localhost-only control HTTP server for the Electron main process:
///   POST /scan   -> trigger an immediate DoH probe/rescan
///   GET  /status -> current DoH + probe ranking + connection stats
pub async fn run_control_server(doh: Arc<DohClient>, discord: Arc<DiscordManager>, port: u16) {
    let bind = format!("127.0.0.1:{}", port);
    let Ok(listener) = TcpListener::bind(&bind).await else {
        eprintln!("[{}] [CTRL] failed to bind {}: {}", now_iso(), bind, std::io::Error::last_os_error());
        return;
    };
    println!("[{}] [CTRL] control server listening on {}", now_iso(), bind);

    loop {
        let Ok((mut socket, _)) = listener.accept().await else {
            continue;
        };
        let doh = Arc::clone(&doh);
        let discord = Arc::clone(&discord);
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
            let path = parts
                .next()
                .unwrap_or("/")
                .split('?')
                .next()
                .unwrap_or("/");

            let body = match (method, path) {
                ("POST", "/scan") => {
                    doh.trigger_rescan("control-api");
                    r#"{"ok":true}"#.to_string()
                }
                ("GET", "/status") => status_json(&doh, &discord),
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

fn status_json(doh: &Arc<DohClient>, discord: &Arc<DiscordManager>) -> String {
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

    json!({
        "ok": true,
        "currentDohIndex": idx,
        "currentDoh": DOH_SERVERS[idx],
        "probeResults": probes,
        "totalSwitches": doh.total_switch_count.load(std::sync::atomic::Ordering::Relaxed),
        "discordBestIp": best_ip,
        "discordBestRtt": best_rtt
    })
    .to_string()
}
