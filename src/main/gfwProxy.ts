/*
 * Vegcord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2026 Vendicated and Vegcord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import { ChildProcess, spawn } from "child_process";
import { app } from "electron";
import { existsSync, mkdirSync } from "fs";
import { dirname, join } from "path";

let proxyProcess: ChildProcess | null = null;

const PROXY_PORT = 4500;
const PROXY_HOST = "127.0.0.1";

function getProxyDir() {
    const candidates: string[] = [];
    if (app.isPackaged) {
        // When packaged, executable files cannot be spawned from inside the
        // asar archive (ENOENT on Windows). They are unpacked via
        // "asarUnpack" in package.json, so look there first.
        candidates.push(join(process.resourcesPath, "app.asar.unpacked", "static", "gfw_proxy"));
        candidates.push(join(process.resourcesPath, "gfw_proxy"));
    }
    // Running from a source checkout or the Arch package (which runs the app
    // unpackaged through the system electron, where app.isPackaged is still
    // true but resourcesPath points at the electron installation)
    candidates.push(join(__dirname, "..", "..", "static", "gfw_proxy"));
    candidates.push("/opt/vegord/static/gfw_proxy");

    for (const candidate of candidates) {
        if (existsSync(candidate)) return candidate;
    }
    return candidates[0];
}

function getProxyBinaryName() {
    return process.platform === "win32" ? "gfw_proxy.exe" : "gfw_proxy";
}

function findRustBinary(dir: string): string | null {
    const binName = getProxyBinaryName();
    const candidate1 = join(dir, binName);
    if (existsSync(candidate1)) return candidate1;

    const candidate2 = join(__dirname, "..", "..", "gfw_proxy_rs", "target", "release", binName);
    if (existsSync(candidate2)) return candidate2;

    const candidate3 = join("/opt/vegord", "static", "gfw_proxy", binName);
    if (existsSync(candidate3)) return candidate3;

    return null;
}

export function startProxy() {
    const dir = getProxyDir();
    const rustBinary = findRustBinary(dir);
    const script = join(dir, "pyprox_HTTPS_v3.0.py");

    const logPrefix = "[GFW Proxy]";

    // Create writable proxy data directory for logs
    const proxyDataDir = join(app.getPath("userData"), "proxy");
    mkdirSync(proxyDataDir, { recursive: true });

    try {
        if (rustBinary) {
            console.log(`${logPrefix} Starting high-performance Rust proxy binary at ${rustBinary}`);
            proxyProcess = spawn(rustBinary, ["--port", String(PROXY_PORT), "--data-dir", proxyDataDir], {
                cwd: dirname(rustBinary),
                stdio: ["ignore", "pipe", "pipe"],
                env: {
                    ...process.env,
                    VEGORD_PROXY_DATA_DIR: proxyDataDir,
                    VEGORD_PROXY_PORT: String(PROXY_PORT)
                }
            });
        } else if (existsSync(script)) {
            console.log(`${logPrefix} Rust binary not found, falling back to Python script at ${script}`);
            proxyProcess = spawn("python3", [script], {
                cwd: dir,
                stdio: ["ignore", "pipe", "pipe"],
                env: { ...process.env, PYTHONUNBUFFERED: "1", VEGORD_PROXY_DATA_DIR: proxyDataDir }
            });
        } else {
            console.error(`${logPrefix} Neither Rust binary nor Python script found in ${dir}. Skipping proxy startup.`);
            return;
        }

        proxyProcess.stdout?.on("data", (data: Buffer) => {
            for (const line of data.toString().trim().split("\n")) {
                if (line) console.log(`${logPrefix} ${line}`);
            }
        });

        proxyProcess.stderr?.on("data", (data: Buffer) => {
            for (const line of data.toString().trim().split("\n")) {
                if (line) console.error(`${logPrefix} ${line}`);
            }
        });

        proxyProcess.on("error", err => {
            console.error(`${logPrefix} Failed to start proxy:`, err.message);
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
