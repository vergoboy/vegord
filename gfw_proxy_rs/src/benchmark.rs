// Fragmentation benchmark (spec section 4.1): actually TRY several
// (num_fragment, sleep_ms) configurations against a real, reachable TLS endpoint
// — routed through the proxy itself — and persist the one that completes
// fastest. The old code just copied the current config values into the preset,
// so the "measured" preset was always identical to the defaults no matter what
// the network tolerated.
//
// Design notes:
// - A real HTTPS request (rustls ClientHello from reqwest) is used instead of a
//   hand-crafted ClientHello, which many servers rejected.
// - The request is routed through the proxy so the fragmentation under test is
//   the exact code path relayed traffic uses.
// - The probe targets the service that actually needs unfragmenting on this
//   network (Discord). On filtered networks the ISP's transparent relay answers
//   the *Cloudflare* SNIs it hijacks (e.g. cloudflare-dns.com) with an HTTP 403
//   even when fragmentation is useless, so those must NOT be used as a probe.
// - Success = any HTTP response reached from the real Cloudflare edge (even a
//   403/1034 security block): that proves the fragmented ClientHello got past
//   the DPI SNI filter and reached the origin's network. Timeout / HTTP 0 means
//   the GFW reset the handshake.
// - The winner is the config with the FEWEST fragments that still succeeds:
//   extra fragments add latency and handshake fragility without benefit once the
//   minimum bypass threshold is met.

use std::sync::Arc;
use std::time::Duration;

use crate::stats::now_iso;

// Configurations to try, roughly monotonic in DPI-evasion strength/cost.
const CANDIDATES: &[(usize, u64)] = &[(1, 0), (3, 1), (6, 1), (12, 4), (20, 10)];

const BENCH_REQUEST_TIMEOUT: Duration = Duration::from_secs(6);
// The relay path is flaky, so each candidate is probed several times and must
// reach the edge on every try to count as usable. Blocked candidates fail fast
// (RST), so the extra tries cost little; if nothing is reliably reachable the
// caller keeps the previously-saved preset instead of churning on noise.
const TRIES_PER_CANDIDATE: usize = 3;
const MIN_OK_TRIES: usize = 3;
// Benchmark probe: the real service that needs SNI-bypass. The ISP's transparent
// relay (Cloudflare Spectrum at 203.32.120.226, offline-DNS-mapped) forwards
// discord.com by SNI, so any HTTP response here proves the fragmentation got the
// ClientHello past the GFW. (The 403/1034 that comes back is Discord's own
// security block and is irrelevant to fragmentation selection.)
const BENCH_URL: &str = "https://discord.com/api/v9/gateway";

/// Run the full fragmentation benchmark through the given proxy address.
/// Returns the best (num_fragment, sleep_ms) among candidates that completed a
/// real TLS handshake, or None when nothing worked (caller keeps current config).
pub async fn benchmark_fragmentation(
    proxy_url: &str,
    frag_override: &Arc<parking_lot::RwLock<Option<(usize, u64)>>>,
) -> Option<(usize, u64)> {
    let client = reqwest::Client::builder()
        .timeout(BENCH_REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .proxy(
            reqwest::Proxy::all(proxy_url)
                .unwrap_or_else(|_| reqwest::Proxy::all("http://127.0.0.1:4500").unwrap()),
        )
        .build()
        .ok()?;

    // (num_fragment, sleep_ms, ok_tries)
    let mut results: Vec<(usize, u64, usize)> = Vec::new();
    for &(n, s) in CANDIDATES {
        let mut ok = 0usize;
        for attempt in 0..TRIES_PER_CANDIDATE {
            *frag_override.write() = Some((n, s));
            let result = tokio::time::timeout(
                BENCH_REQUEST_TIMEOUT,
                client.get(BENCH_URL).send(),
            )
            .await;
            *frag_override.write() = None;

            let status = match result {
                Ok(Ok(resp)) => resp.status().as_u16(),
                _ => 0,
            };
            if status != 0 {
                ok += 1;
                println!(
                    "[{}] [PRESET] frag benchmark {}x{}ms try {}/{} -> OK (HTTP {})",
                    now_iso(),
                    n,
                    s,
                    attempt + 1,
                    TRIES_PER_CANDIDATE,
                    status
                );
            } else {
                println!(
                    "[{}] [PRESET] frag benchmark {}x{}ms try {}/{} -> blocked (HTTP {})",
                    now_iso(),
                    n,
                    s,
                    attempt + 1,
                    TRIES_PER_CANDIDATE,
                    status
                );
            }
        }
        println!(
            "[{}] [PRESET] frag benchmark {}x{}ms -> {}/{} succeeded",
            now_iso(),
            n,
            s,
            ok,
            TRIES_PER_CANDIDATE
        );
        if ok >= MIN_OK_TRIES {
            results.push((n, s, ok));
        }
    }

    // Most reliable config wins; ties broken toward fewer fragments (least
    // handshake overhead once the bypass threshold is reliably met).
    results.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)));
    results.first().map(|r| (r.0, r.1))
}
