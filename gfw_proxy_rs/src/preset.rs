use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::benchmark::benchmark_fragmentation;
use crate::config::{Config, DOH_SERVERS};
use crate::discord::DiscordManager;
use crate::doh::DohClient;
use crate::stats::now_iso;

// Smart ISP-aware preset system (spec section 4).
//
// On every start the proxy benchmarks this network (DoH resolvers ranked by
// measured RTT, fragmentation parameters, Discord voice IP RTT+loss) and stores
// a local preset keyed by an ISP fingerprint (ASN + country, never the raw IP).
// When a validated server-side preset exists for the same fingerprint it is
// fetched and applied instead — returning users skip the cold benchmark. The
// whole point is that an ISP's idiosyncrasies (which IPs/parameters work) are
// learned once and reused.

const PRESET_SCHEMA_VERSION: u32 = 1;
const FINGERPRINT_REFRESH_SEC: u64 = 15 * 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FragmentationPreset {
    pub num_fragment: usize,
    pub fragment_sleep_ms: u64,
    pub split_strategy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordIpScore {
    pub rtt_ms: Option<f64>,
    pub loss_pct: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preset {
    pub schema_version: u32,
    pub isp_fingerprint: String,
    pub generated_at: String,
    pub confidence: String,
    pub fragmentation: FragmentationPreset,
    pub doh_resolvers_ranked: Vec<String>,
    pub discord_ip_scores: HashMap<String, DiscordIpScore>,
    pub voice_port_timeout_sec: u64,
    pub socket_timeout_sec: u64,
}

fn presets_dir(config: &Config) -> PathBuf {
    config.data_dir.join("presets")
}

fn current_preset_path(config: &Config) -> PathBuf {
    presets_dir(config).join("current.json")
}

fn sanitize_fingerprint(f: &str) -> String {
    f.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// Apply a previously saved preset (local benchmark or downloaded server
/// preset) to the runtime config at boot, so a known-good configuration is
/// active immediately without re-benchmarking.
pub fn apply_preset_to_config(config: &mut Config) {
    let path = current_preset_path(config);
    let Ok(text) = fs::read_to_string(&path) else {
        return;
    };
    let Ok(preset) = serde_json::from_str::<Preset>(&text) else {
        return;
    };
    if preset.schema_version != PRESET_SCHEMA_VERSION {
        return;
    }
    if preset.fragmentation.num_fragment >= 1 {
        config.num_fragment = preset.fragmentation.num_fragment;
    }
    config.fragment_sleep_ms = preset.fragmentation.fragment_sleep_ms;
    if preset.socket_timeout_sec >= 1 {
        config.socket_timeout_sec = preset.socket_timeout_sec;
    }
    if preset.voice_port_timeout_sec >= 1 {
        config.udp_loss_window_sec = preset.voice_port_timeout_sec;
    }
    // Seed the DoH start index from the last saved ranking so a known-good
    // resolver (e.g. one that was only reachable via index 7 last run) is
    // tried first on startup instead of always burning timeouts on
    // DOH_SERVERS[0..] in order. Missing/unknown entries fall back to index 0.
    if let Some(top) = preset.doh_resolvers_ranked.first() {
        config.preferred_doh_index = DOH_SERVERS.iter().position(|u| u == top);
    }
    println!(
        "[{}] [PRESET] applied {} (frag={}x{}ms, confidence={})",
        now_iso(),
        path.display(),
        config.num_fragment,
        config.fragment_sleep_ms,
        preset.confidence
    );
}

/// Background sync loop: fingerprint -> benchmark -> save local preset ->
/// fetch/apply server preset -> upload (consent-gated). Re-checks the
/// fingerprint periodically and re-benchmarks when the ISP changes.
pub async fn run_preset_sync(
    mut config: Config,
    doh: Arc<DohClient>,
    discord: Arc<DiscordManager>,
    frag_override: Arc<parking_lot::RwLock<Option<(usize, u64)>>>,
) {
    if !config.preset_sync_enabled {
        return;
    }
    // Let the startup DoH probe and the first pings settle before measuring.
    tokio::time::sleep(Duration::from_secs(3)).await;

    let client = build_panel_client(&config);
    let mut last_fingerprint = String::new();

    loop {
        let fingerprint = fetch_fingerprint(&config, &client).await;
        if fingerprint != last_fingerprint {
            last_fingerprint = fingerprint.clone();

            // Wait a bounded time for the DoH flow to populate the Discord IP
            // pool, then run a fresh ping round so the preset carries real
            // RTT/loss scores instead of the all-null snapshot the old code
            // captured ~3s after boot (before the first 30s ping interval).
            // If no Discord traffic happened yet, resolve a few core domains
            // directly so the pool has candidates to ping.
            let deadline = Instant::now() + Duration::from_secs(12);
            if discord.count_ips() == 0 {
                for host in ["discord.com", "gateway.discord.gg", "cdn.discordapp.com"] {
                    if doh.query(host).await.is_some() {
                        break;
                    }
                }
            }
            while discord.count_ips() == 0 && Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            discord.ping_once(&config).await;

            // Actually benchmark fragmentation against a reachable TLS target
            // and select the fastest config that still completes a handshake.
            // Previously the preset just re-saved whatever the defaults were.
            // With an upstream SOCKS5 relay configured, Discord no longer uses
            // the fragmented path at all, so the benchmark is skipped.
            let proxy_url = format!("http://127.0.0.1:{}", config.listen_port);
            let mut bench_ok = false;
            if config.relay_socks5.is_none() {
                if let Some((n, s)) = benchmark_fragmentation(&proxy_url, &frag_override).await {
                    bench_ok = true;
                    config.num_fragment = n;
                    config.fragment_sleep_ms = s;
                    // Leave the override in place for the rest of this session so
                    // the measured config takes effect on live relayed traffic
                    // immediately, not only after the next process restart (the
                    // shared knob makes the proxy read it via current_frag()).
                    *frag_override.write() = Some((n, s));
                    println!(
                        "[{}] [PRESET] benchmark selected fragmentation {}x{}ms",
                        now_iso(),
                        n,
                        s
                    );
                }
            }

            // Only claim a measurement when one actually happened. With an
            // upstream relay the fragmented path is bypassed entirely (nothing
            // to measure); when the benchmark fails 0/3 for every candidate the
            // saved values are just defaults or a stale preset, not a finding.
            let confidence = if config.relay_socks5.is_some() {
                "relay"
            } else if bench_ok {
                "measured"
            } else {
                "unknown"
            };
            let measured = build_measured_preset(&config, &doh, &discord, &fingerprint, confidence);
            save_measured_preset(&config, &measured);

            // Prefer a validated, aggregated server preset over our single
            // measurement; fall back to the local benchmark.
            let mut applied = measured.clone();
            if let Some(mut p) = fetch_server_preset(&config, &client, &fingerprint).await {
                p.confidence = "downloaded".to_string();
                println!(
                    "[{}] [PRESET] downloaded server preset for {}",
                    now_iso(),
                    fingerprint
                );
                applied = p;
            }
            save_current_preset(&config, &applied);
            upload_preset(&config, &client, &measured).await;
        }
        tokio::time::sleep(Duration::from_secs(FINGERPRINT_REFRESH_SEC)).await;
    }
}

/// Route panel traffic through the proxy itself so preset sync keeps working on
/// a filtered network (same reasoning as the DoH client).
fn build_panel_client(config: &Config) -> Client {
    let proxy_url = format!("http://127.0.0.1:{}", config.listen_port);
    Client::builder()
        .timeout(Duration::from_secs(config.panel_timeout_sec))
        .proxy(
            reqwest::Proxy::all(&proxy_url)
                .unwrap_or_else(|_| reqwest::Proxy::all("http://127.0.0.1:4500").unwrap()),
        )
        .build()
        .unwrap_or_else(|_| Client::new())
}

/// Fingerprint = ASN + country, never the raw public IP. The panel resolves the
/// public IP server-side and returns only the sanitized ASN/country pair, so no
/// IP leaves the client. Falls back to "unknown" when the lookup fails.
async fn fetch_fingerprint(config: &Config, client: &Client) -> String {
    let base = config.panel_base_url.trim_end_matches('/');
    let url = format!("{}/ipinfo", base);
    let fut = client.get(&url).send();
    match tokio::time::timeout(Duration::from_secs(config.panel_timeout_sec), fut).await {
        Ok(Ok(resp)) if resp.status().is_success() => {
            match resp.json::<serde_json::Value>().await {
                Ok(v) => {
                    let asn = v
                        .get("asn")
                        .and_then(|x| {
                            x.as_str()
                                .map(str::to_string)
                                .or_else(|| x.as_i64().map(|n| n.to_string()))
                        })
                        .unwrap_or_default()
                        .trim()
                        .to_string();
                    let country = v
                        .get("country")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    if !asn.is_empty() {
                        return format!("AS{}-{}", asn.trim_start_matches("AS"), country.to_uppercase());
                    }
                    println!("[{}] [PRESET] ipinfo response missing asn", now_iso());
                }
                Err(e) => println!("[{}] [PRESET] ipinfo parse error: {}", now_iso(), e),
            }
        }
        Ok(Ok(resp)) => {
            println!(
                "[{}] [PRESET] ipinfo returned HTTP {}",
                now_iso(),
                resp.status()
            );
        }
        Ok(Err(e)) => println!("[{}] [PRESET] ipinfo error: {}", now_iso(), e),
        Err(_) => println!("[{}] [PRESET] ipinfo timeout", now_iso()),
    }
    "unknown".to_string()
}

fn build_measured_preset(
    config: &Config,
    doh: &Arc<DohClient>,
    discord: &Arc<DiscordManager>,
    fingerprint: &str,
    confidence: &str,
) -> Preset {
    // DoH resolvers ranked by the most recent probe pass, then the static list
    // as a stable tiebreaker for servers that have not been probed yet.
    let probe_results = doh.probe_results.read();
    let mut ranked: Vec<String> = probe_results
        .iter()
        .filter(|r| r.avg_rtt_ms.is_some())
        .map(|r| r.url.to_string())
        .collect();
    for url in DOH_SERVERS {
        let u = url.to_string();
        if !ranked.contains(&u) {
            ranked.push(u);
        }
    }
    drop(probe_results);

    let mut scores = HashMap::new();
    for (ip, rtt, loss) in discord.get_ip_scores() {
        scores.insert(
            ip.to_string(),
            DiscordIpScore {
                rtt_ms: rtt,
                loss_pct: loss,
            },
        );
    }

    Preset {
        schema_version: PRESET_SCHEMA_VERSION,
        isp_fingerprint: fingerprint.to_string(),
        generated_at: now_iso(),
        confidence: confidence.to_string(),
        fragmentation: FragmentationPreset {
            num_fragment: config.num_fragment,
            fragment_sleep_ms: config.fragment_sleep_ms,
            split_strategy: "sni_offset".to_string(),
        },
        doh_resolvers_ranked: ranked,
        discord_ip_scores: scores,
        voice_port_timeout_sec: config.udp_loss_window_sec,
        socket_timeout_sec: config.socket_timeout_sec,
    }
}

fn save_preset(config: &Config, preset: &Preset, name: &str) -> Option<PathBuf> {
    let dir = presets_dir(config);
    if fs::create_dir_all(&dir).is_err() {
        return None;
    }
    let path = dir.join(name);
    let Ok(json) = serde_json::to_string_pretty(preset) else {
        return None;
    };
    match fs::write(&path, &json) {
        Ok(()) => {
            println!(
                "[{}] [PRESET] saved {} (fingerprint={}, confidence={})",
                now_iso(),
                path.display(),
                preset.isp_fingerprint,
                preset.confidence
            );
            Some(path)
        }
        Err(e) => {
            println!("[{}] [PRESET] failed to save {}: {}", now_iso(), path.display(), e);
            None
        }
    }
}

fn save_measured_preset(config: &Config, preset: &Preset) {
    let name = format!("{}.json", sanitize_fingerprint(&preset.isp_fingerprint));
    save_preset(config, preset, &name);
}

fn save_current_preset(config: &Config, preset: &Preset) {
    save_preset(config, preset, "current.json");
}

/// Fetch the aggregated, validated preset for this fingerprint (if any). Only
/// presets with the current schema version are trusted.
async fn fetch_server_preset(config: &Config, client: &Client, fingerprint: &str) -> Option<Preset> {
    if fingerprint == "unknown" {
        return None;
    }
    let base = config.panel_base_url.trim_end_matches('/');
    let url = format!("{}/preset/{}", base, fingerprint);
    let fut = client.get(&url).send();
    let resp = tokio::time::timeout(Duration::from_secs(config.panel_timeout_sec), fut)
        .await
        .ok()?
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let v: serde_json::Value = resp.json().await.ok()?;
    let p = v.get("preset")?;
    let preset: Preset = serde_json::from_value(p.clone()).ok()?;
    if preset.schema_version != PRESET_SCHEMA_VERSION {
        return None;
    }
    Some(preset)
}

/// Consent-gated upload: no-ops unless VEGORD_PANEL_UPLOAD_TOKEN is configured.
async fn upload_preset(config: &Config, client: &Client, preset: &Preset) {
    if config.panel_upload_token.is_empty() || preset.isp_fingerprint == "unknown" {
        return;
    }
    let base = config.panel_base_url.trim_end_matches('/');
    let url = format!("{}/preset", base);
    let body = json!({
        "preset": preset,
        "fingerprint": preset.isp_fingerprint
    });
    let fut = client
        .post(&url)
        .header("content-type", "application/json")
        .header("x-upload-token", &config.panel_upload_token)
        .body(body.to_string())
        .send();
    match tokio::time::timeout(Duration::from_secs(config.panel_timeout_sec), fut).await {
        Ok(Ok(resp)) => {
            println!(
                "[{}] [PRESET] upload for {} -> {}",
                now_iso(),
                preset.isp_fingerprint,
                resp.status()
            );
        }
        Ok(Err(e)) => println!("[{}] [PRESET] upload error: {}", now_iso(), e),
        Err(_) => println!("[{}] [PRESET] upload timeout", now_iso()),
    }
}
