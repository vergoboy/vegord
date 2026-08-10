/*
 * vegord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2026 Vendicated and vegord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import { app } from "electron";
import { createWriteStream, mkdirSync, WriteStream } from "fs";
import { join } from "path";

// Local debug log: appends everything worth debugging to
// <userData>/logs/main.log. Unlike connectionLog (which uploads a condensed
// summary to the panel), this file keeps full detail: every renderer console
// line, every proxy packet/connection event and periodic internet-quality
// snapshots. Nothing here ever leaves the machine.

let stream: WriteStream | null = null;

// Ring buffer of the most recent formatted lines, so the debug window can
// show history immediately on open without reading the whole file.
const recentLines: string[] = [];
const MAX_RECENT_LINES = 5000;

// Live subscribers (the debug window) get every new line as it is written.
const lineListeners = new Set<(line: string) => void>();

export function onFileLogLine(listener: (line: string) => void) {
    lineListeners.add(listener);
    return () => lineListeners.delete(listener);
}

export function getRecentFileLogLines(): string[] {
    return [...recentLines];
}

export function clearFileLog() {
    recentLines.length = 0;
    try {
        stream?.end();
        const dir = join(app.getPath("userData"), "logs");
        mkdirSync(dir, { recursive: true });
        stream = createWriteStream(join(dir, "main.log"), { flags: "w" });
        stream.on("error", () => {});
    } catch {
        // best effort
    }
}

function pad(n: number, width = 2) {
    return String(n).padStart(width, "0");
}

function timestamp() {
    const d = new Date();
    const tzo = -d.getTimezoneOffset();
    const sign = tzo >= 0 ? "+" : "-";
    const abs = Math.abs(tzo);
    return (
        `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T` +
        `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}.${pad(d.getMilliseconds(), 3)}` +
        `${sign}${pad(Math.floor(abs / 60))}:${pad(abs % 60)}`
    );
}

export function initFileLog() {
    try {
        const dir = join(app.getPath("userData"), "logs");
        mkdirSync(dir, { recursive: true });
        stream = createWriteStream(join(dir, "main.log"), { flags: "a" });
        stream.on("error", () => {});
        fileLog("MAIN", "info", `log session started v${app.getVersion()}`);
    } catch {
        // Logging is best-effort; never let it take the app down.
    }
}

export function fileLog(tag: string, level: string, message: string) {
    if (!stream) return;
    try {
        const line = `[${timestamp()}] [${tag}:${level}] ${String(message).slice(0, 4000)}`;
        stream.write(line + "\n");
        recentLines.push(line);
        if (recentLines.length > MAX_RECENT_LINES) recentLines.splice(0, recentLines.length - MAX_RECENT_LINES);
        for (const listener of lineListeners) {
            try {
                listener(line);
            } catch {
                // a broken listener must not break logging
            }
        }
    } catch {
        // best effort
    }
}

export function closeFileLog() {
    try {
        stream?.end();
    } catch {}
    stream = null;
}
