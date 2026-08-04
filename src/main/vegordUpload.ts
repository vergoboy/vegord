/*
 * Vegcord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2026 Vendicated and Vegcord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import { dialog } from "electron";
import { createReadStream } from "fs";
import { stat } from "fs/promises";
import { request } from "https";
import { basename } from "path";

import { IpcEvents } from "../shared/IpcEvents";
import { logError, logInfo, logWarn } from "./connectionLog";
import { mainWin } from "./mainWindow";
import { handle } from "./utils/ipcWrappers";

// Vegcord file sharing: uploads land in /opt/vegord/files on the server and are
// served back at <base>/vegord/files/<name>.
const UPLOAD_URL = "https://vergoboy.ir/vegord/api/v1/upload";
const UPLOAD_TOKEN = "56b7674d6d8ae1635bf60741a9ebd3a44896549ddcf3b870";
const MAX_UPLOAD_BYTES = 100 * 1024 * 1024;
const UPLOAD_TIMEOUT_MS = 5 * 60 * 1000;

function sendProgress(sent: number, total: number) {
    const win = mainWin;
    if (win && !win.isDestroyed()) {
        win.webContents.send(IpcEvents.VEGORD_UPLOAD_PROGRESS, { sent, total });
    }
}

function sanitizeName(name: string) {
    return (
        name
            .replace(/[\\/]/g, "")
            .slice(0, 120)
            .replace(/[^\w.\-() ]/g, "_")
            .trim() || "file"
    );
}

function uploadFile(filePath: string, fileName: string, size: number): Promise<string> {
    return new Promise((resolve, reject) => {
        const url = new URL(UPLOAD_URL);
        url.searchParams.set("name", sanitizeName(fileName));

        const req = request(
            url,
            {
                method: "POST",
                headers: {
                    "x-upload-token": UPLOAD_TOKEN,
                    "content-type": "application/octet-stream",
                    "content-length": size
                }
            },
            res => {
                let body = "";
                res.setEncoding("utf8");
                res.on("data", chunk => (body += chunk));
                res.on("end", () => {
                    if (res.statusCode && res.statusCode >= 200 && res.statusCode < 300) {
                        try {
                            const parsed = JSON.parse(body) as { url?: string };
                            if (parsed.url) return resolve(parsed.url);
                        } catch {}
                        reject(new Error("invalid server response"));
                    } else {
                        reject(new Error(`upload failed (${res.statusCode}) ${body.slice(0, 200)}`));
                    }
                });
            }
        );
        req.on("error", reject);
        req.setTimeout(UPLOAD_TIMEOUT_MS, () => req.destroy(new Error("upload timed out")));

        const stream = createReadStream(filePath);
        let sent = 0;
        stream.on("data", (chunk: Buffer) => {
            sent += chunk.length;
            sendProgress(sent, size);
        });
        stream.on("error", reject);
        stream.pipe(req);
    });
}

handle(IpcEvents.VEGORD_UPLOAD, async () => {
    const win = mainWin;
    if (!win || win.isDestroyed()) return { error: "no window" };

    const { canceled, filePaths } = await dialog.showOpenDialog(win, {
        title: "Upload Vegord",
        buttonLabel: "Upload",
        properties: ["openFile"]
    });
    if (canceled || !filePaths.length) return { canceled: true };

    const filePath = filePaths[0];
    let size: number;
    try {
        const st = await stat(filePath);
        if (!st.isFile()) return { error: "not a file" };
        size = st.size;
    } catch {
        return { error: "cannot read file" };
    }

    if (size > MAX_UPLOAD_BYTES) {
        return { error: "file larger than 100 MB" };
    }
    if (size === 0) {
        return { error: "file is empty" };
    }

    const fileName = basename(filePath);
    sendProgress(0, size);
    logInfo(`upload_start name=${fileName} size=${size}`);
    try {
        const url = await uploadFile(filePath, fileName, size);
        sendProgress(size, size);
        logInfo(`upload_ok ${url}`);
        return { url, name: fileName, size };
    } catch (err) {
        const { message } = err as Error;
        logWarn(`upload_failed ${message}`);
        logError("upload_failed");
        return { error: message };
    }
});
