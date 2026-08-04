mod config;
mod control;
mod discord;
mod doh;
mod fragment;
mod preset;
mod proxy;
mod stats;

use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use config::Config;
use discord::DiscordManager;
use doh::DohClient;
use proxy::ProxyServer;
use stats::StatsManager;
use tokio::net::TcpListener;

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
    if let Ok(port_str) = env::var("VEGORD_PROXY_CONTROL_PORT") {
        if let Ok(p) = port_str.parse::<u16>() {
            config.control_port = p;
        }
    } else {
        config.control_port = config.listen_port + 1;
    }
    if let Ok(token) = env::var("VEGORD_PROXY_CONTROL_TOKEN") {
        config.control_token = token;
    }
    if let Ok(v) = env::var("VEGORD_PRESET_SYNC") {
        config.preset_sync_enabled = v == "1" || v.eq_ignore_ascii_case("true");
    }
    if let Ok(base) = env::var("VEGORD_PANEL_BASE") {
        config.panel_base_url = base;
    }
    if let Ok(token) = env::var("VEGORD_PANEL_UPLOAD_TOKEN") {
        config.panel_upload_token = token;
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
            "--control-token" if i + 1 < args.len() => {
                config.control_token = args[i + 1].clone();
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
            "--preset-sync" if i + 1 < args.len() => {
                config.preset_sync_enabled = args[i + 1] == "1" || args[i + 1].eq_ignore_ascii_case("true");
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

    // Apply the last known-good preset (local benchmark or downloaded server
    // preset) before anything else touches the config.
    preset::apply_preset_to_config(&mut config);

    println!(
        "[{}] [INIT] Vegord Rust GFW Proxy v3.2.0 starting on port {} (data dir: {})",
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

    // Bind the proxy listener BEFORE spawning dependent tasks so the startup
    // DoH probe can route through the proxy immediately. Bound to 127.0.0.1
    // only: the Electron main process is the sole consumer (spec section 3,
    // security). Exposing a forward proxy on 0.0.0.0 would open it to the LAN.
    let bind_addr = format!("127.0.0.1:{}", config.listen_port);
    let listener = TcpListener::bind(&bind_addr).await?;
    println!(
        "[{}] [START] Rust GFW Proxy Listening on {}",
        stats::now_iso(),
        bind_addr
    );

    // Startup DoH probe: scan all candidates, rank by measured RTT and select
    // the most stable lowest-ping server as the active one.
    let doh_probe = Arc::clone(&doh_client);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        doh_probe.probe_and_select().await;
    });

    // Localhost control API for the Electron main process.
    let ctrl_doh = Arc::clone(&doh_client);
    let ctrl_discord = Arc::clone(&discord_mgr);
    let ctrl_token = config.control_token.clone();
    tokio::spawn(async move {
        control::run_control_server(ctrl_doh, ctrl_discord, config.control_port, &ctrl_token).await;
    });

    // ISP-aware preset sync: benchmark this network, save a local preset,
    // fetch/apply a validated server preset when available (spec section 4).
    let preset_doh = Arc::clone(&doh_client);
    let preset_discord = Arc::clone(&discord_mgr);
    let preset_cfg = config.clone();
    tokio::spawn(async move {
        preset::run_preset_sync(preset_cfg, preset_doh, preset_discord).await;
    });

    // Start proxy server
    let proxy_server = ProxyServer::new(
        config,
        doh_client,
        discord_mgr,
        stats_mgr,
    );

    proxy_server.run(listener).await
}
