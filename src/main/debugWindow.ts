/*
 * vegord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2026 Vendicated and vegord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import { BrowserWindow, clipboard, ipcMain } from "electron";
import { join } from "path";
import { IpcEvents } from "shared/IpcEvents";

import { getProxyStatus, requestProxyRescan } from "./dohControl";
import { clearFileLog, getRecentFileLogLines, onFileLogLine } from "./fileLog";
import { loadView } from "./vegordStatic";

// Debug panel: a small window showing live proxy/DoH/internet status plus a
// streaming view of the local debug log, for diagnosing connectivity issues.

let debugWin: BrowserWindow | null = null;

export function isDebugWindowOpen(): boolean {
    return debugWin !== null && !debugWin.isDestroyed();
}

export function toggleDebugWindow() {
    if (isDebugWindowOpen()) {
        debugWin!.close();
        debugWin = null;
    } else {
        createDebugWindow();
    }
}

function createDebugWindow() {
    if (isDebugWindowOpen()) return;

    debugWin = new BrowserWindow({
        width: 720,
        height: 760,
        minWidth: 520,
        minHeight: 400,
        title: "vegord Debug Panel",
        backgroundColor: "#0d0a14",
        autoHideMenuBar: true,
        webPreferences: {
            nodeIntegration: false,
            contextIsolation: true,
            sandbox: true,
            devTools: true,
            preload: join(__dirname, "debugPreload.js")
        }
    });

    debugWin.on("closed", () => {
        debugWin = null;
    });

    loadView(debugWin, "debug.html");
}

function broadcastLogLine(line: string) {
    if (!isDebugWindowOpen()) return;
    const wc = debugWin!.webContents;
    if (wc.isDestroyed()) return;
    wc.send(IpcEvents.DEBUG_LOG_LINE, line);
}

onFileLogLine(broadcastLogLine);

ipcMain.handle(IpcEvents.DEBUG_GET_STATUS, () => getProxyStatus());
ipcMain.handle(IpcEvents.DEBUG_GET_LOGS, () => getRecentFileLogLines());
ipcMain.handle(IpcEvents.DEBUG_CLEAR_LOG, () => {
    clearFileLog();
    return true;
});
ipcMain.handle(IpcEvents.DEBUG_RESCAN, () => requestProxyRescan());
ipcMain.handle(IpcEvents.DEBUG_COPY, (_event, text: string) => {
    if (typeof text !== "string" || text.length === 0) return false;
    clipboard.writeText(text);
    return true;
});
