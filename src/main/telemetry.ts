/*
 * Vegcord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2026 Vendicated and Vegcord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import { randomUUID } from "crypto";
import { app, Notification, shell } from "electron";
import { readFileSync } from "fs";
import { join } from "path";
import { IpcEvents } from "shared/IpcEvents";

import { Settings, State } from "./settings";
import { handle } from "./utils/ipcWrappers";

// Vegord Panel (https://vergoboy.ir/vegord/)
export const PANEL_BASE = "https://vergoboy.ir/vegord/api/v1";

const HEARTBEAT_INTERVAL = 15 * 60 * 1000;

const GITHUB_API_URL = "https://api.github.com/repos/vergoboy/vegord/releases/latest";
const GITHUB_RELEASES_URL = "https://github.com/vergoboy/vegord/releases";
const GITHUB_CHECK_INTERVAL = 6 * 60 * 60 * 1000;

const REQUEST_TIMEOUT = 15_000;

let discordUsername: string | null = null;
let networkInfo: { effectiveType?: string; downlink?: number; rtt?: number } | null = null;
let heartbeatInFlight = false;

let cachedVersion: string | null = null;

// In this packaging the app runs as `electron dist/js/main.js` (not a packaged
// .app), so app.getVersion() reports Electron's version instead of ours.
// Read the real version from our package.json instead.
export function getAppVersion(): string {
    if (cachedVersion) return cachedVersion;
    try {
        const pkg = JSON.parse(readFileSync(join(__dirname, "..", "..", "package.json"), "utf8")) as {
            version?: string;
        };
        cachedVersion = String(pkg.version || "0.0.0");
    } catch {
        cachedVersion = app.getVersion();
    }
    return cachedVersion;
}

function telemetryEnabled() {
    return Settings.store.enableTelemetry !== false;
}

export function getClientId(): string {
    State.store.telemetry ??= {};
    State.store.telemetry.clientId ??= randomUUID();
    return State.store.telemetry.clientId!;
}

export async function fetchWithTimeout(url: string, init: RequestInit, timeoutMs = REQUEST_TIMEOUT) {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), timeoutMs);
    try {
        return await fetch(url, { ...init, signal: controller.signal });
    } finally {
        clearTimeout(timer);
    }
}

export async function sendHeartbeat() {
    if (!telemetryEnabled() || heartbeatInFlight) return;
    heartbeatInFlight = true;
    try {
        const payload = {
            clientId: getClientId(),
            version: getAppVersion(),
            platform: process.platform,
            arch: process.arch,
            network: networkInfo,
            ...(Settings.store.shareDiscordUsername !== false && discordUsername ? { discordUsername } : {})
        };
        const res = await fetchWithTimeout(PANEL_BASE + "/heartbeat", {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify(payload)
        });
        if (!res.ok) console.warn(`[Telemetry] heartbeat failed: ${res.status}`);
    } catch (e) {
        console.warn(`[Telemetry] heartbeat error: ${e instanceof Error ? e.message : e}`);
    } finally {
        heartbeatInFlight = false;
    }
}

async function checkForUpdates() {
    try {
        const res = await fetchWithTimeout(GITHUB_API_URL, {
            headers: {
                accept: "application/vnd.github+json",
                "user-agent": `vegord/${getAppVersion()}`
            }
        });
        if (!res.ok) return;
        const release = await res.json();
        const latest = String(release.tag_name || "").replace(/^v/i, "");
        const current = getAppVersion();
        if (!latest || latest === current || !isNewerVersion(latest, current)) return;

        const { updater } = State.store;
        if (updater?.ignoredVersion === latest) return;
        if ((updater?.snoozeUntil ?? 0) > Date.now()) return;

        const notif = new Notification({
            title: "Vegcord update available",
            body: `Version ${latest} is now available. Click to open the download page.`,
            silent: false
        });
        notif.on("click", () => shell.openExternal(`${GITHUB_RELEASES_URL}/tag/${release.tag_name}`));
        notif.show();
    } catch {}
}

function isNewerVersion(a: string, b: string): boolean {
    const pa = a.split(/[.-]/).map(n => parseInt(n, 10) || 0);
    const pb = b.split(/[.-]/).map(n => parseInt(n, 10) || 0);
    const len = Math.max(pa.length, pb.length);
    for (let i = 0; i < len; i++) {
        const x = pa[i] ?? 0;
        const y = pb[i] ?? 0;
        if (x !== y) return x > y;
    }
    return false;
}

export function startTelemetry() {
    handle(IpcEvents.TELEMETRY_SET_USER, (_e, username: string | null) => {
        const next = username || null;
        if (next === discordUsername) return;
        discordUsername = next;
        if (telemetryEnabled()) sendHeartbeat();
    });

    handle(
        IpcEvents.TELEMETRY_SET_NETWORK,
        (_e, network: { effectiveType?: string; downlink?: number; rtt?: number }) => {
            if (!network || typeof network !== "object") return;
            networkInfo = {
                effectiveType:
                    typeof network.effectiveType === "string" ? network.effectiveType.slice(0, 16) : undefined,
                downlink:
                    typeof network.downlink === "number" && isFinite(network.downlink) ? network.downlink : undefined,
                rtt: typeof network.rtt === "number" && isFinite(network.rtt) ? network.rtt : undefined
            };
        }
    );

    if (!telemetryEnabled()) return;

    sendHeartbeat();
    setInterval(sendHeartbeat, HEARTBEAT_INTERVAL);
}

export function startGithubUpdateChecker() {
    checkForUpdates();
    setInterval(checkForUpdates, GITHUB_CHECK_INTERVAL);
}
