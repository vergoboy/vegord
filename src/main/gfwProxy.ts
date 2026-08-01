/*
 * Vegcord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2026 Vendicated and Vegcord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import { ChildProcess, execFile, spawn } from "child_process";
import { app } from "electron";
import { existsSync, mkdirSync } from "fs";
import { dirname, join } from "path";

import { logError, logInfo, logWarn } from "./connectionLog";

let proxyProcess: ChildProcess | null = null;

const PROXY_PORT = 4500;
const PROXY_HOST = "127.0.0.1";
const CONTROL_PORT = 4501;

// Lines from the Rust proxy that are worth uploading to the panel's connection log.
const PROXY_LOG_MARKERS = [
    "[INIT]",
    "[START]",
    "[CTRL]",
    "[DoH SWITCH]",
    "[DoH BLACKLIST]",
    "[DoH PROBE]",
    "[DoH TIMEOUT]",
    "[DoH WARN]",
    "[DoH ERR]",
    "[DNS FAIL]",
    "[FILTERED]",
    "[DISCORD]"
];

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

function spawnProxy() {
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
            logInfo(`proxy_start rust binary=${rustBinary}`);
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
            logWarn("proxy_fallback python script");
            proxyProcess = spawn("python3", [script], {
                cwd: dir,
                stdio: ["ignore", "pipe", "pipe"],
                env: { ...process.env, PYTHONUNBUFFERED: "1", VEGORD_PROXY_DATA_DIR: proxyDataDir }
            });
        } else {
            console.error(
                `${logPrefix} Neither Rust binary nor Python script found in ${dir}. Skipping proxy startup.`
            );
            logError("proxy_missing binary and script");
            return;
        }

        proxyProcess.stdout?.on("data", (data: Buffer) => {
            for (const line of data.toString().trim().split("\n")) {
                if (!line) continue;
                console.log(`${logPrefix} ${line}`);
                if (PROXY_LOG_MARKERS.some(m => line.includes(m))) logInfo(`proxy ${line.trim()}`);
            }
        });

        proxyProcess.stderr?.on("data", (data: Buffer) => {
            for (const line of data.toString().trim().split("\n")) {
                if (line) {
                    console.error(`${logPrefix} ${line}`);
                    logWarn(`proxy_stderr ${line.trim()}`);
                }
            }
        });

        proxyProcess.on("error", err => {
            console.error(`${logPrefix} Failed to start proxy:`, err.message);
            logError(`proxy_error ${err.message}`);
        });

        proxyProcess.on("exit", (code, signal) => {
            console.log(`${logPrefix} Exited (code=${code}, signal=${signal})`);
            logWarn(`proxy_exit code=${code} signal=${signal}`);
            proxyProcess = null;
        });

        console.log(`${logPrefix} Started on ${PROXY_HOST}:${PROXY_PORT}`);
    } catch (err) {
        console.error(`${logPrefix} Failed to launch proxy:`, err);
    }
}

export function startProxy() {
    spawnProxy();
}

export function stopProxy() {
    if (proxyProcess) {
        console.log("[GFW Proxy] Stopping...");
        proxyProcess.kill("SIGTERM");
        proxyProcess = null;
    }
}

// The app must never run without a working proxy, so every startup verifies
// that our proxy is actually answering before Discord is loaded. If the port
// is held by a leftover process (e.g. an orphaned proxy from a killed
// session) it is freed and the proxy restarted.
export async function ensureProxyRunning(): Promise<boolean> {
    if (await isProxyHealthy()) return true;

    for (let attempt = 0; attempt < 3; attempt++) {
        if (!proxyProcess) {
            await freeStaleProxyPorts();
            spawnProxy();
        }

        if (await waitForHealthy(4000)) return true;

        // Proxy failed to come up (e.g. "Address already in use") — free the
        // port and try again with a fresh process.
        logWarn(`proxy_unhealthy attempt=${attempt + 1}`);
        await stopProxy();
        await freeStaleProxyPorts();
    }

    return false;
}

async function isProxyHealthy(): Promise<boolean> {
    try {
        const res = await fetch(`http://127.0.0.1:${CONTROL_PORT}/status`, {
            signal: AbortSignal.timeout(1500)
        });
        return res.ok;
    } catch {
        return false;
    }
}

function waitForHealthy(timeoutMs: number): Promise<boolean> {
    return new Promise(resolve => {
        const startedAt = Date.now();
        const check = async () => {
            if (await isProxyHealthy()) return resolve(true);
            if (Date.now() - startedAt >= timeoutMs) return resolve(false);
            setTimeout(check, 250);
        };
        check();
    });
}

async function freeStaleProxyPorts(): Promise<void> {
    for (const port of [PROXY_PORT, CONTROL_PORT]) {
        try {
            const pids = await findPidsOnPort(port);
            for (const pid of pids) {
                if (pid === process.pid) continue;
                console.log(`[GFW Proxy] Killing stale process ${pid} holding port ${port}`);
                logWarn(`proxy_stale_kill pid=${pid} port=${port}`);
                try {
                    process.kill(pid, "SIGKILL");
                } catch {}
            }
        } catch {
            // best effort
        }
    }

    // Also try fuser, which handles the process-group juggling for us.
    try {
        await new Promise<void>(resolve => {
            execFile("bash", ["-c", `fuser -k ${PROXY_PORT}/tcp ${CONTROL_PORT}/tcp 2>/dev/null || true`], () =>
                resolve()
            );
        });
    } catch {
        // best effort
    }
}

function findPidsOnPort(port: number): Promise<number[]> {
    return new Promise(resolve => {
        execFile("ss", ["-ltnp"], (err, stdout) => {
            if (err) return resolve([]);

            const pids: number[] = [];
            const portToken = new RegExp(`:${port}(\\s|$)`);
            for (const line of stdout.split("\n")) {
                if (!portToken.test(line)) continue;
                // format 1: users:(("gfw_proxy",pid=189422,fd=21))
                const pidMatch = line.match(/pid=(\d+)/);
                // format 2: users:(("gfw_proxy"189422fd=21))
                const altMatch = line.match(/"([^"]+)"(\d+)/);
                const pid = pidMatch?.[1] ?? altMatch?.[2];
                if (pid) pids.push(Number(pid));
            }
            resolve(pids.filter(n => Number.isFinite(n) && n > 0));
        });
    });
}

export function getProxyAddress() {
    return `socks5://${PROXY_HOST}:${PROXY_PORT}`;
}

export function getCustomProxyAddress() {
    const idx = process.argv.indexOf("--proxy-server");
    if (idx !== -1 && idx + 1 < process.argv.length) {
        return process.argv[idx + 1];
    }
    // also check the Electron switch format
    return app.commandLine.getSwitchValue("proxy-server") || null;
}
