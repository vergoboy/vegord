/*
 * Vegcord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2026 Vendicated and Vegcord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import { randomUUID } from "crypto";

import { Settings } from "./settings";
import { fetchWithTimeout, getAppVersion, getClientId, PANEL_BASE } from "./telemetry";

// Client-side connection log: buffered, batched upload to the Vegord panel so
// the proxy/connection health (startup, DoH events, reconnect, load failures)
// can be analysed server-side to improve DoH/proxy behaviour per network/ISP.

const MAX_BATCH = 100;
const FLUSH_INTERVAL = 10 * 1000;
const MIN_FLUSH_GAP = 5 * 1000;

interface LogEntry {
    t: number;
    level: "info" | "warn" | "error";
    msg: string;
}

const sessionId = randomUUID();
const buffer: LogEntry[] = [];
let lastFlush = 0;
let flushing = false;

function log(level: LogEntry["level"], msg: string) {
    if (Settings.store.enableTelemetry === false) return;
    buffer.push({ t: Date.now(), level, msg: String(msg).slice(0, 1200) });
    if (buffer.length >= MAX_BATCH) flush();
}

export function logInfo(msg: string) {
    log("info", msg);
}

export function logWarn(msg: string) {
    log("warn", msg);
}

export function logError(msg: string) {
    log("error", msg);
}

export async function flush() {
    if (flushing || buffer.length === 0) return;
    const now = Date.now();
    if (now - lastFlush < MIN_FLUSH_GAP) return;
    const batch = buffer.splice(0, MAX_BATCH);
    flushing = true;
    try {
        const res = await fetchWithTimeout(`${PANEL_BASE}/logs`, {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({
                clientId: getClientId(),
                sessionId,
                version: getAppVersion(),
                platform: process.platform,
                arch: process.arch,
                logs: batch
            })
        });
        if (res.ok) lastFlush = now;
        else buffer.unshift(...batch);
    } catch {
        buffer.unshift(...batch);
    } finally {
        flushing = false;
    }
}

export function startConnectionLog() {
    setInterval(flush, FLUSH_INTERVAL);
}

// Ensure buffered logs are sent before the app quits.
export function flushBeforeQuit() {
    if (buffer.length === 0) return;
    const batch = buffer.splice(0, MAX_BATCH);
    const payload = JSON.stringify({
        clientId: getClientId(),
        sessionId,
        version: getAppVersion(),
        platform: process.platform,
        arch: process.arch,
        logs: batch
    });
    try {
        // best-effort synchronous-ish send using keep-alive
        fetch(`${PANEL_BASE}/logs`, {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: payload,
            keepalive: true
        }).catch(() => {});
    } catch {
        /* ignore */
    }
}
