/*
 * Vesktop, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2026 Vendicated and Vesktop contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import { ChildProcess, spawn } from "child_process";
import { app } from "electron";
import { existsSync } from "fs";
import { join } from "path";

let proxyProcess: ChildProcess | null = null;

const PROXY_PORT = 4500;
const PROXY_HOST = "127.0.0.1";

function getProxyDir() {
    if (app.isPackaged) {
        return join(process.resourcesPath, "gfw_proxy");
    }
    const staticDir = join(__dirname, "..", "..", "static", "gfw_proxy");
    if (existsSync(staticDir)) {
        return staticDir;
    }
    return join(__dirname, "..", "..", "gfw_resist_HTTPS_proxy");
}

export function startProxy() {
    const dir = getProxyDir();
    const script = join(dir, "pyprox_HTTPS_v3.0.py");

    const logPrefix = "[GFW Proxy]";

    if (!existsSync(script)) {
        console.error(`${logPrefix} Script not found at ${script}. Skipping proxy startup.`);
        return;
    }

    try {
        proxyProcess = spawn("python3", [script], {
            cwd: dir,
            stdio: ["ignore", "pipe", "pipe"],
            env: { ...process.env, PYTHONUNBUFFERED: "1" }
        });

        proxyProcess.stdout?.on("data", (data: Buffer) => {
            for (const line of data.toString().trim().split("\n")) {
                console.log(`${logPrefix} ${line}`);
            }
        });

        proxyProcess.stderr?.on("data", (data: Buffer) => {
            for (const line of data.toString().trim().split("\n")) {
                console.error(`${logPrefix} ${line}`);
            }
        });

        proxyProcess.on("error", err => {
            console.error(`${logPrefix} Failed to start proxy:`, err.message);
            console.error(`${logPrefix} Make sure python3, dnspython, and requests are installed`);
        });

        proxyProcess.on("exit", (code, signal) => {
            console.log(`${logPrefix} Exited (code=${code}, signal=${signal})`);
            proxyProcess = null;
        });

        console.log(`${logPrefix} Started on ${PROXY_HOST}:${PROXY_PORT}`);
    } catch (err) {
        console.error(`${logPrefix} Failed to launch proxy:`, err);
    }
}

export function stopProxy() {
    if (proxyProcess) {
        console.log("[GFW Proxy] Stopping...");
        proxyProcess.kill("SIGTERM");
        proxyProcess = null;
    }
}

export function getProxyAddress() {
    return `socks5://${PROXY_HOST}:${PROXY_PORT}`;
}

export function isProxyDisabled() {
    return process.argv.includes("--no-proxy");
}

export function getCustomProxyAddress() {
    const idx = process.argv.indexOf("--proxy-server");
    if (idx !== -1 && idx + 1 < process.argv.length) {
        return process.argv[idx + 1];
    }
    // also check the Electron switch format
    return app.commandLine.getSwitchValue("proxy-server") || null;
}
