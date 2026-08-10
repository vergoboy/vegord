/*
 * vegord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2026 Vendicated and vegord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import { logInfo } from "./connectionLog";
import { fileLog } from "./fileLog";

// Localhost control API exposed by the Rust gfw_proxy (binds 127.0.0.1:4501).
// Lets the Electron main process read the live DoH selection / probe ranking
// and trigger a manual rescan.

export const PROXY_CONTROL_PORT = 4501;

export interface DohProbeResultJson {
    index: number;
    url: string;
    avgRttMs: number | null;
    successes: number;
    failures: number;
}

export interface DiscordIpScoreJson {
    ip: string;
    rttMs: number | null;
    lossPct: number | null;
}

export interface ProxyStatus {
    ok: boolean;
    currentDohIndex: number;
    currentDoh: string;
    probeResults: DohProbeResultJson[];
    totalSwitches: number;
    discordBestIp: string | null;
    discordBestRtt: number | null;
    connections?: { total: number; ok: number; filtered: number };
    queries?: { total: number; ok: number; fail: number };
    traffic?: { ulBytes: number; dlBytes: number };
    discordIps?: DiscordIpScoreJson[];
}

const REQUEST_TIMEOUT = 2000;

async function controlRequest(path: string, method = "GET"): Promise<ProxyStatus | null> {
    try {
        const controller = new AbortController();
        const timer = setTimeout(() => controller.abort(), REQUEST_TIMEOUT);
        try {
            const res = await fetch(`http://127.0.0.1:${PROXY_CONTROL_PORT}${path}`, {
                method,
                signal: controller.signal
            });
            if (!res.ok) return null;
            return (await res.json()) as ProxyStatus;
        } finally {
            clearTimeout(timer);
        }
    } catch {
        return null;
    }
}

export async function getProxyStatus(): Promise<ProxyStatus | null> {
    return controlRequest("/status");
}

export async function requestProxyRescan(): Promise<boolean> {
    const res = await controlRequest("/scan", "POST");
    return res?.ok === true;
}

export function shortProbeSummary(status: ProxyStatus | null): string {
    if (!status) return "doh_status unavailable";
    const best = status.probeResults
        .filter(r => r.avgRttMs != null)
        .slice(0, 3)
        .map(r => `${r.index}#${Math.round(r.avgRttMs!)}ms`)
        .join(", ");
    const hosts = status.probeResults.filter(r => r.avgRttMs == null).map(r => r.index);
    const fails = hosts.length ? ` fail#${hosts.join(",")}` : "";
    return (
        `doh_status current=#${status.currentDohIndex} switches=${status.totalSwitches} ` +
        `top=${best || "none"}${fails} best_ip=${status.discordBestIp ?? "-"}`
    );
}

function fmtBytes(bytes: number | undefined): string {
    if (bytes == null) return "-";
    if (bytes < 1024) return `${bytes}B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)}KB`;
    return `${(bytes / 1024 / 1024).toFixed(2)}MB`;
}

// Full internet-quality snapshot (for the local debug log): per-IP Discord
// RTT + packet loss, proxy connection health, DoH query health and traffic.
export function qualitySummary(status: ProxyStatus | null): string {
    if (!status) return "quality unavailable";

    const conns = status.connections;
    const connStr = conns ? `conn=${conns.total} ok=${conns.ok} filtered=${conns.filtered}` : "conn=-";

    const { queries } = status;
    const queryStr = queries ? `doh_query=${queries.total} ok=${queries.ok} fail=${queries.fail}` : "doh_query=-";

    const { traffic } = status;
    const trafficStr = traffic ? `ul=${fmtBytes(traffic.ulBytes)} dl=${fmtBytes(traffic.dlBytes)}` : "traffic=-";

    const ipList = (status.discordIps ?? [])
        .map(s => {
            const rtt = s.rttMs != null ? `${Math.round(s.rttMs)}ms` : "?ms";
            const loss = s.lossPct != null ? `${s.lossPct.toFixed(1)}%loss` : "?loss";
            return `${s.ip}:${rtt}/${loss}`;
        })
        .join(" ");
    const ips = status.discordBestIp
        ? `best_ip=${status.discordBestIp} rtt=${status.discordBestRtt?.toFixed(0) ?? "?"}ms`
        : "best_ip=-";
    const ipDetails = ipList ? ` ips[${ipList}]` : "";

    return `${connStr} ${queryStr} ${trafficStr} ${ips}${ipDetails}`;
}

export function logProxyStatus(reason: string) {
    getProxyStatus()
        .then(status => {
            logInfo(`${reason} ${shortProbeSummary(status)} ${qualitySummary(status)}`);
        })
        .catch(() => {});
}

// Internet-quality snapshots into the local debug log (much more frequent than
// the 15-min panel uploads, and with full per-IP detail).
const QUALITY_INTERVAL = 60 * 1000;
let qualityTimer: NodeJS.Timeout | null = null;

export function startQualitySnapshot() {
    if (qualityTimer) return;
    const tick = () => {
        getProxyStatus()
            .then(status => {
                fileLog("NETQUALITY", "info", `snapshot ${shortProbeSummary(status)} ${qualitySummary(status)}`);
            })
            .catch(() => {});
    };
    tick();
    qualityTimer = setInterval(tick, QUALITY_INTERVAL);
    qualityTimer.unref?.();
}
