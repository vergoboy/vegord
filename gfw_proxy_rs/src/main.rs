mod config;
mod discord;
mod doh;
mod fragment;
mod proxy;
mod stats;

use std::env;
use std::path::PathBuf;
use std::sync::Arc;

use config::Config;
use discord::DiscordManager;
use doh::DohClient;
use proxy::ProxyServer;
use stats::StatsManager;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let mut config = Config::default();

    // Environment variables
    if let Ok(dir) = env::var("VEGORD_PROXY_DATA_DIR") {
        config.data_dir = PathBuf::from(dir);
    }
    if let Ok(port_str) = env::var("VEGORD_PROXY_PORT") {
        if let Ok(p) = port_str.parse::<u16>() {
            config.listen_port = p;
        }
    }

    // CLI argument parsing
    let args: Vec<String> = env::args().collect();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--port" | "-p" if i + 1 < args.len() => {
                if let Ok(p) = args[i + 1].parse::<u16>() {
                    config.listen_port = p;
                }
                i += 2;
            }
            "--data-dir" | "-d" if i + 1 < args.len() => {
                config.data_dir = PathBuf::from(&args[i + 1]);
                i += 2;
            }
            "--num-fragment" if i + 1 < args.len() => {
                if let Ok(n) = args[i + 1].parse::<usize>() {
                    config.num_fragment = n;
                }
                i += 2;
            }
            "--fragment-sleep" if i + 1 < args.len() => {
                if let Ok(ms) = args[i + 1].parse::<u64>() {
                    config.fragment_sleep_ms = ms;
                }
                i += 2;
            }
            _ => {
                i += 1;
            }
        }
    }

    if !config.data_dir.exists() {
        let _ = std::fs::create_dir_all(&config.data_dir);
    }

    println!(
        "[{}] [INIT] Vegord Rust GFW Proxy v3.0.0 starting on port {} (data dir: {})",
        stats::now_iso(),
        config.listen_port,
        config.data_dir.display()
    );

    let discord_mgr = DiscordManager::new();
    let doh_client = DohClient::new(config.clone(), Arc::clone(&discord_mgr));
    let stats_mgr = StatsManager::new(config.clone(), Arc::clone(&doh_client), Arc::clone(&discord_mgr));

    // Start background Discord IP benchmarking & ping loop
    discord_mgr.clone().start_pinger(config.clone());

    // Start background traffic & health log writer task
    stats_mgr.clone().start_log_writer();

    // Start proxy server
    let proxy_server = ProxyServer::new(
        config,
        doh_client,
        discord_mgr,
        stats_mgr,
    );

    proxy_server.run().await
}
