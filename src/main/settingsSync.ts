/*
 * Vegcord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2026 Vendicated and Vegcord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import { app } from "electron";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "fs";
import { dirname } from "path";
import { IpcEvents } from "shared/IpcEvents";
import type { State as TState } from "shared/settings";
import { deflateSync, inflateSync } from "zlib";

import { VENCORD_QUICKCSS_FILE, VENCORD_SETTINGS_FILE } from "./constants";
import { Settings, State } from "./settings";
import { fetchWithTimeout, PANEL_BASE } from "./telemetry";
import { handle } from "./utils/ipcWrappers";

// Replacement for Vencord Cloud. Every startup silently saves this user's
// settings to the Vegord panel; the first login of the day on a client that
// already has a sync secret restores the remote settings first. The panel
// issues a random per-user secret on the first sync and the client stores it
// locally, so a brand-new device cannot pull until the user syncs from a
// trusted device (or gets a fresh secret from the panel admin and starts the
// app once with --sync-secret=<code>).

const REQUEST_TIMEOUT = 15_000;

let currentUser: { id: string; username: string } | null = null;
let syncing = false;
let pushTimer: NodeJS.Timeout | undefined;

function todayStr(): string {
    const d = new Date();
    const mm = String(d.getMonth() + 1).padStart(2, "0");
    const dd = String(d.getDate()).padStart(2, "0");
    return `${d.getFullYear()}-${mm}-${dd}`;
}

function syncEntry(): NonNullable<TState["settingsSync"]> {
    return (State.store.settingsSync ??= {});
}

function userEntry(userId: string): { secret?: string; lastRestoreDay?: string } {
    const sync = syncEntry();
    return (sync.users ??= {})[userId] ?? {};
}

function collectSettings() {
    let vencord = {};
    try {
        vencord = JSON.parse(readFileSync(VENCORD_SETTINGS_FILE, "utf8"));
    } catch {}

    let quickCss = "";
    try {
        quickCss = readFileSync(VENCORD_QUICKCSS_FILE, "utf8");
    } catch {}

    const vegord = JSON.parse(JSON.stringify(Settings.plain ?? {}));

    return { vencord, quickCss, vegord };
}

// Settings are deflated (zlib) and base64-encoded on the wire so even large
// QuickCSS files stay small. Both ends use Node's built-in zlib.
function encodeSettings(s: object): string {
    return deflateSync(Buffer.from(JSON.stringify(s))).toString("base64");
}

function decodeSettings(b64: unknown): Record<string, unknown> | null {
    if (typeof b64 !== "string" || !b64) return null;
    try {
        return JSON.parse(inflateSync(Buffer.from(b64, "base64")).toString("utf8"));
    } catch {
        return null;
    }
}

async function register(): Promise<string | null> {
    const res = await fetchWithTimeout(
        PANEL_BASE + "/settings/register",
        {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({ userId: currentUser!.id, username: currentUser!.username })
        },
        REQUEST_TIMEOUT
    );
    if (res.status === 409) return null;
    if (!res.ok) throw new Error(`register failed: ${res.status}`);
    const data = await res.json();
    return typeof data.secret === "string" ? data.secret : null;
}

async function save(secret: string) {
    const res = await fetchWithTimeout(
        PANEL_BASE + "/settings",
        {
            method: "PUT",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({
                userId: currentUser!.id,
                secret,
                settings: encodeSettings(collectSettings())
            })
        },
        REQUEST_TIMEOUT
    );
    if (!res.ok) throw new Error(`save failed: ${res.status}`);
}

async function restore(secret: string): Promise<boolean> {
    const res = await fetchWithTimeout(
        `${PANEL_BASE}/settings?userId=${encodeURIComponent(currentUser!.id)}`,
        { headers: { authorization: `Bearer ${secret}` } },
        REQUEST_TIMEOUT
    );
    if (res.status === 404) return false;
    if (res.status === 401) throw new Error("restore unauthorized (bad sync secret)");
    if (!res.ok) throw new Error(`restore failed: ${res.status}`);

    const data = await res.json();
    const s = decodeSettings(data?.settings);
    if (!s || typeof s !== "object") return false;

    if (s.vencord && typeof s.vencord === "object") {
        try {
            mkdirSync(dirname(VENCORD_SETTINGS_FILE), { recursive: true });
            writeFileSync(VENCORD_SETTINGS_FILE, JSON.stringify(s.vencord, null, 4));
        } catch {}
    }
    if (typeof s.quickCss === "string") {
        try {
            mkdirSync(dirname(VENCORD_QUICKCSS_FILE), { recursive: true });
            writeFileSync(VENCORD_QUICKCSS_FILE, s.quickCss);
        } catch {}
    }
    if (s.vegord && typeof s.vegord === "object") {
        try {
            Settings.setData(s.vegord);
        } catch {}
    }

    return true;
}

async function syncNow() {
    if (!currentUser || syncing) return;
    syncing = true;
    try {
        const sync = syncEntry();
        const userId = currentUser.id;
        const rec = userEntry(userId);

        // Bootstrap for a brand-new device: the user is given a secret by the
        // panel admin and starts the app once with --sync-secret=<code>.
        if (!rec.secret && sync.pendingSecret) {
            rec.secret = sync.pendingSecret;
            delete sync.pendingSecret;
            sync.users![userId] = rec;
        }

        if (!rec.secret) {
            const secret = await register();
            if (!secret) {
                console.warn(
                    "[SettingsSync] This user already has a sync account on the panel, but this device has no secret. " +
                        "Ask the panel admin for a sync secret and start the app with --sync-secret=<code> once."
                );
                return;
            }
            rec.secret = secret;
            sync.users![userId] = rec;
        }

        // First login of the day on this client: restore before saving so the
        // freshly restored settings are what gets uploaded afterwards.
        if (rec.lastRestoreDay !== todayStr()) {
            rec.lastRestoreDay = todayStr();
            sync.users![userId] = rec;

            const restored = await restore(rec.secret);
            if (restored) {
                await save(rec.secret);
                app.relaunch();
                app.exit(0);
                return;
            }
        }

        await save(rec.secret);
    } catch (e) {
        console.warn(`[SettingsSync] error: ${e instanceof Error ? e.message : e}`);
    } finally {
        syncing = false;
    }
}

export function startSettingsSync() {
    handle(IpcEvents.SYNC_SET_USER, (_e, user: { id?: unknown; username?: unknown } | null) => {
        if (!user || typeof user.id !== "string" || !user.id) return;
        const id = user.id.slice(0, 128);
        const username = typeof user.username === "string" ? user.username.slice(0, 64) : "";
        if (currentUser?.id === id) return;
        currentUser = { id, username };
        syncNow();
    });

    const secretArg = process.argv.find(a => a.startsWith("--sync-secret="));
    if (secretArg) {
        const pending = secretArg.slice("--sync-secret=".length).trim();
        if (pending) {
            const sync = syncEntry();
            sync.pendingSecret = pending;
            if (currentUser) syncNow();
        }
    }
}

// Debounced save after the user changes any setting (Settings.setData / IPC).
export function requestSettingsSync() {
    if (!currentUser) return;
    clearTimeout(pushTimer);
    pushTimer = setTimeout(syncNow, 2_000);
}

// Ensure the quickCss file exists so readFileSync never throws on first boot.
if (!existsSync(VENCORD_QUICKCSS_FILE)) {
    try {
        mkdirSync(dirname(VENCORD_QUICKCSS_FILE), { recursive: true });
        writeFileSync(VENCORD_QUICKCSS_FILE, "");
    } catch {}
}
