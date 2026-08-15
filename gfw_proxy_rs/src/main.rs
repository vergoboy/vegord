mod benchmark;
mod config;
mod control;
mod discord;
mod doh;
mod fragment;
mod mitm;
mod preset;
mod proxy;
mod stats;
mod tun;

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
    if let Ok(spec) = env::var("VEGORD_RELAY_SOCKS5") {
        if let Some(relay) = config::parse_relay_socks5(&spec) {
            config.relay_socks5 = Some(relay);
        }
    }
    if let Ok(v) = env::var("VEGORD_TLS_MITM") {
        config.tls_mitm = v == "1" || v.eq_ignore_ascii_case("true");
    }
    if let Ok(v) = env::var("VEGORD_TUN_SPLIT") {
        config.tun_split_enabled = v == "1" || v.eq_ignore_ascii_case("true");
    }
    if let Ok(bin) = env::var("VEGORD_TUN2PROXY_BIN") {
        config.tun2proxy_bin = bin;
    }
    if let Ok(name) = env::var("VEGORD_TUN_NAME") {
        config.tun_name = name;
    }
    if let Ok(v) = env::var("VEGORD_RELAY_IDLE_TIMEOUT") {
        if let Ok(sec) = v.parse::<u64>() {
            config.relay_idle_timeout_sec = sec;
        }
    }

    // Apply the last known-good preset (local benchmark or downloaded server
    // preset) BEFORE CLI parsing so explicit user flags always win over the
    // preset (spec section 4 principle #6: server/local config is a starting
    // point, never forced).
    preset::apply_preset_to_config(&mut config);

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
            "--relay-socks5" if i + 1 < args.len() => {
                if let Some(relay) = config::parse_relay_socks5(&args[i + 1]) {
                    config.relay_socks5 = Some(relay);
                }
                i += 2;
            }
            "--tls-mitm" => {
                config.tls_mitm = true;
                i += 1;
            }
            "--tun-split" => {
                config.tun_split_enabled = true;
                i += 1;
            }
            "--tun-name" if i + 1 < args.len() => {
                config.tun_name = args[i + 1].clone();
                i += 2;
            }
            "--tun-fwmark" if i + 1 < args.len() => {
                if let Ok(m) = args[i + 1].parse::<u32>() {
                    config.tun_fwmark = m;
                }
                i += 2;
            }
            "--tun-table" if i + 1 < args.len() => {
                if let Ok(t) = args[i + 1].parse::<u32>() {
                    config.tun_table = t;
                }
                i += 2;
            }
            "--tun2proxy-bin" if i + 1 < args.len() => {
                config.tun2proxy_bin = args[i + 1].clone();
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

    if let Some(relay) = &config.relay_socks5 {
        println!(
            "[{}] [RELAY] Discord routed via upstream SOCKS5 {}:{}",
            stats::now_iso(),
            relay.host,
            relay.port
        );
    }
    if config.tls_mitm {
        println!(
            "[{}] [MITM] Local TLS MITM enabled for Discord hosts (app must run with --ignore-certificate-errors)",
            stats::now_iso()
        );
    }

    println!(
        "[{}] [INIT] Vegord Rust GFW Proxy v3.6.0 starting on port {} (data dir: {})",
        stats::now_iso(),
        config.listen_port,
        config.data_dir.display()
    );

    let discord_mgr = DiscordManager::new();
    let doh_client = DohClient::new(config.clone(), Arc::clone(&discord_mgr));
    let stats_mgr = StatsManager::new(config.clone(), Arc::clone(&doh_client), Arc::clone(&discord_mgr));
    let tun_mgr = tun::TunManager::new(&config);
    let mitm_mgr = mitm::MitmManager::new();

    // Start background Discord IP benchmarking & ping loop
    discord_mgr.clone().start_pinger(config.clone());

    // Start background traffic & health log writer task
    stats_mgr.clone().start_log_writer();

    // Split-tunnel (tun2proxy) for Discord-only traffic, when enabled.
    if config.tun_split_enabled {
        let tun_start = Arc::clone(&tun_mgr);
        let tun_discord = Arc::clone(&discord_mgr);
        let tun_port = config.listen_port;
        tokio::spawn(async move {
            tun_start.start(tun_discord, tun_port).await;
        });
    }

    // Clean shutdown: tear down the split tunnel (routes, rules, tun2proxy
    // child) before exiting so Discord is never left pointed at a dead TUN.
    let tun_sig = Arc::clone(&tun_mgr);
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            if let Ok(mut sigterm) =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            {
                let _ = sigterm.recv().await;
                tun_sig.stop().await;
                std::process::exit(0);
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
            tun_sig.stop().await;
            std::process::exit(0);
        }
    });
    let tun_ctrl_c = Arc::clone(&tun_mgr);
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        tun_ctrl_c.stop().await;
        std::process::exit(0);
    });

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

    // Fragmentation override knob, shared between the proxy, the preset
    // benchmark and the control API. The benchmark tries several
    // (num_fragment, sleep_ms) configs on real relayed traffic via this knob;
    // POST /frag lets the Electron main (or a user) override it live.
    let frag_override: Arc<parking_lot::RwLock<Option<(usize, u64)>>> =
        Arc::new(parking_lot::RwLock::new(None));

    // Localhost control API for the Electron main process.
    let ctrl_doh = Arc::clone(&doh_client);
    let ctrl_discord = Arc::clone(&discord_mgr);
    let ctrl_stats = Arc::clone(&stats_mgr);
    let ctrl_tun = Arc::clone(&tun_mgr);
    let ctrl_frag = Arc::clone(&frag_override);
    let ctrl_token = config.control_token.clone();
    tokio::spawn(async move {
        control::run_control_server(
            ctrl_doh,
            ctrl_discord,
            ctrl_stats,
            ctrl_tun,
            ctrl_frag,
            config.control_port,
            &ctrl_token,
        )
        .await;
    });

    // ISP-aware preset sync: benchmark this network, save a local preset,
    // fetch/apply a validated server preset when available (spec section 4).
    let preset_doh = Arc::clone(&doh_client);
    let preset_discord = Arc::clone(&discord_mgr);
    let preset_cfg = config.clone();
    let preset_knob = Arc::clone(&frag_override);
    tokio::spawn(async move {
        preset::run_preset_sync(preset_cfg, preset_doh, preset_discord, preset_knob).await;
    });

    // Start proxy server
    let proxy_server = ProxyServer::new(
        config,
        doh_client,
        discord_mgr,
        stats_mgr,
        frag_override,
        tun_mgr,
        mitm_mgr,
    );

    proxy_server.run(listener).await
}
