/*
 * Vegcord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2026 Vendicated and Vegcord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import { logInfo } from "./connectionLog";

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

export interface ProxyStatus {
    ok: boolean;
    currentDohIndex: number;
    currentDoh: string;
    probeResults: DohProbeResultJson[];
    totalSwitches: number;
    discordBestIp: string | null;
    discordBestRtt: number | null;
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

export function logProxyStatus(reason: string) {
    getProxyStatus()
        .then(status => {
            logInfo(`${reason} ${shortProbeSummary(status)}`);
        })
        .catch(() => {});
}
