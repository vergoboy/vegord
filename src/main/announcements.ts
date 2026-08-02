/*
 * Vegcord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2026 Vendicated and Vegcord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import { shell } from "electron";

import { fetchWithTimeout, getClientId, PANEL_BASE } from "./telemetry";

const POLL_INTERVAL = 5 * 60 * 1000;

interface Announcement {
    id: string;
    title: string;
    body: string;
    url: string | null;
}

const queue: Announcement[] = [];
const knownIds = new Set<string>();
let currentAnn: Announcement | null = null;
let currentWin: Electron.BrowserWindow | null = null;

const CONSOLE_DISMISS = "vegordAnnouncement:dismiss";
const CONSOLE_OPEN = "vegordAnnouncement:open";

async function fetchAnnouncements() {
    try {
        const clientId = getClientId();
        const res = await fetchWithTimeout(
            `${PANEL_BASE}/notifications?clientId=${encodeURIComponent(clientId)}`,
            {}
        );
        if (!res.ok) return;
        const data = await res.json();
        const notifications = Array.isArray(data.notifications) ? data.notifications : [];

        let added = false;
        for (const n of notifications) {
            if (!n?.id || knownIds.has(n.id)) continue;
            knownIds.add(n.id);
            queue.push({ id: n.id, title: n.title || "Vegcord", body: n.body || "", url: n.url || null });
            added = true;
        }
        if (added) showNext(currentWin);
    } catch {}
}

function bindConsoleMessages(win: Electron.BrowserWindow) {
    win.webContents.on("console-message", event => {
        const msg = event.message;
        if (msg === CONSOLE_DISMISS) dismiss();
        else if (msg === CONSOLE_OPEN) openUrl();
    });
}

function bannerScript(ann: Announcement): string {
    return `(() => {
        const DATA = ${JSON.stringify(ann)};
        const ID = "vegord-announcement";
        if (document.getElementById(ID)) return;
        const wrap = document.createElement("div");
        wrap.id = ID;
        wrap.style.cssText = "position:fixed;right:16px;bottom:16px;z-index:999999;width:380px;max-width:calc(100vw - 32px);background:#171a23;color:#e6e9f0;border:1px solid #2a2f3e;border-radius:10px;padding:14px;box-shadow:0 8px 24px rgba(0,0,0,.45);font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,sans-serif;user-select:none;box-sizing:border-box;";
        const header = document.createElement("div");
        header.style.cssText = "display:flex;align-items:center;margin-bottom:8px;";
        const brand = document.createElement("strong");
        brand.style.cssText = "font-size:12px;color:#8b93a7;letter-spacing:.3px;";
        brand.textContent = "Vegcord Announcement";
        const x = document.createElement("button");
        x.textContent = "\\u00d7";
        x.style.cssText = "margin-left:auto;background:none;border:none;color:#8b93a7;font-size:16px;cursor:pointer;padding:0 4px;line-height:1;";
        header.appendChild(brand);
        header.appendChild(x);
        const title = document.createElement("div");
        title.style.cssText = "font-weight:600;font-size:14px;margin-bottom:6px;";
        title.textContent = DATA.title;
        const body = document.createElement("div");
        body.style.cssText = "font-size:13px;line-height:1.5;color:#c9cedb;white-space:pre-wrap;word-break:break-word;max-height:180px;overflow-y:auto;";
        body.textContent = DATA.body;
        const btns = document.createElement("div");
        btns.style.cssText = "display:flex;gap:8px;margin-top:12px;";
        const mkBtn = (label, primary) => {
            const b = document.createElement("button");
            b.textContent = label;
            b.style.cssText = "flex:1;padding:8px 12px;border-radius:8px;border:none;font-weight:600;font-size:13px;cursor:pointer;color:#fff;background:" + (primary ? "#5865f2" : "#1e2230") + ";border:1px solid #2a2f3e;";
            return b;
        };
        const signal = act => { try { console.log(act); } catch (_) {} };
        const finish = act => { wrap.remove(); signal(act); };
        x.addEventListener("click", () => finish(${JSON.stringify(CONSOLE_DISMISS)}));
        const got = mkBtn("Got it", true);
        got.addEventListener("click", () => finish(${JSON.stringify(CONSOLE_DISMISS)}));
        btns.appendChild(got);
        if (DATA.url) {
            const open = mkBtn("Open", false);
            open.addEventListener("click", () => finish(${JSON.stringify(CONSOLE_OPEN)}));
            btns.insertBefore(open, got);
        }
        wrap.appendChild(header);
        wrap.appendChild(title);
        wrap.appendChild(body);
        wrap.appendChild(btns);
        document.body.appendChild(wrap);
    })();`;
}

function inject(win: Electron.BrowserWindow, ann: Announcement) {
    if (win.isDestroyed() || win.webContents.isDestroyed()) return;
    try {
        win.webContents.executeJavaScript(bannerScript(ann));
    } catch {}
}

function showNext(win: Electron.BrowserWindow | null) {
    if (currentAnn || !win || win.isDestroyed()) return;
    const next = queue.shift();
    if (!next) return;
    currentAnn = next;
    currentWin = win;
    inject(win, next);
}

function dismiss() {
    if (!currentAnn) return;
    ack(currentAnn.id);
    currentAnn = null;
    currentWin = null;
    showNext(currentWin);
}

function openUrl() {
    if (!currentAnn) return;
    if (currentAnn.url) shell.openExternal(currentAnn.url);
    ack(currentAnn.id);
    currentAnn = null;
    currentWin = null;
    showNext(currentWin);
}

async function ack(id: string) {
    try {
        await fetchWithTimeout(`${PANEL_BASE}/notifications/${encodeURIComponent(id)}/ack`, {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({ clientId: getClientId() })
        });
    } catch {}
}

export function bindAnnouncementsToWindow(win: Electron.BrowserWindow) {
    bindConsoleMessages(win);

    // Re-inject after any reload so an undismissed announcement stays visible
    win.webContents.on("did-finish-load", () => {
        if (currentAnn) inject(win, currentAnn);
        else showNext(win);
    });

    showNext(win);
}

export function startAnnouncements() {
    fetchAnnouncements();
    setInterval(fetchAnnouncements, POLL_INTERVAL);
}
