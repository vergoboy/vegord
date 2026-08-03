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

// The child that confirmed it bound the control port, by printing
// "[CTRL] control server listening on 127.0.0.1:<port>". The Rust proxy only
// prints that line AFTER TcpListener::bind succeeded, so it is an authoritative
// "we own the control port" signal that works even where the netstat PID lookup
// is unreliable (Windows).
let controlBindConfirmedBy: ChildProcess | null = null;

// Last-known startup failure details, surfaced to the user in the "Vegcord
// requires the GFW proxy" dialog so the reason is actionable instead of a
// generic "could not start its network proxy".
let lastSpawnError: string | null = null;
let lastChildExit: string | null = null;
let lastDiagnostics: string | null = null;

export function getLastProxyDiagnostics(): string | null {
    return lastDiagnostics;
}

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

    // A new child has not (yet) confirmed it bound the control port.
    controlBindConfirmedBy = null;

    let child: ChildProcess | null = null;
    try {
        if (rustBinary) {
            console.log(`${logPrefix} Starting high-performance Rust proxy binary at ${rustBinary}`);
            logInfo(`proxy_start rust binary=${rustBinary}`);
            child = spawn(rustBinary, ["--port", String(PROXY_PORT), "--data-dir", proxyDataDir], {
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
            // On Windows the interpreter is usually "python", not "python3"
            const pythonBin = process.platform === "win32" ? "python" : "python3";
            child = spawn(pythonBin, [script], {
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

        proxyProcess = child;

        child.stdout?.on("data", (data: Buffer) => {
            for (const line of data.toString().trim().split("\n")) {
                if (!line) continue;
                console.log(`${logPrefix} ${line}`);
                if (line.includes("[CTRL] control server listening")) controlBindConfirmedBy = child;
                if (PROXY_LOG_MARKERS.some(m => line.includes(m))) logInfo(`proxy ${line.trim()}`);
            }
        });

        child.stderr?.on("data", (data: Buffer) => {
            for (const line of data.toString().trim().split("\n")) {
                if (line) {
                    console.error(`${logPrefix} ${line}`);
                    logWarn(`proxy_stderr ${line.trim()}`);
                }
            }
        });

        child.on("error", err => {
            lastSpawnError = err.message;
            console.error(`${logPrefix} Failed to start proxy:`, err.message);
            logError(`proxy_error ${err.message}`);
            // Only clear our reference if this is still the current child, so a
            // fast-exiting stale child can't clobber a freshly spawned one.
            if (proxyProcess === child) proxyProcess = null;
        });

        child.on("exit", (code, signal) => {
            lastChildExit = `code=${code} signal=${signal}`;
            console.log(`${logPrefix} Exited (${lastChildExit})`);
            logWarn(`proxy_exit ${lastChildExit}`);
            if (proxyProcess === child) proxyProcess = null;
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
// that our proxy is actually answering before Discord is loaded.
//
// The control-port answer is ONLY trusted when it is served by a process we
// spawned in this app instance. A stale/orphaned proxy (e.g. from a crashed or
// force-killed session) can still answer /status on the control port while
// being unable to proxy any traffic — trusting it makes Discord load through a
// dead proxy and the app look permanently broken until a manual restart.
// Instead, an unhealthy or foreign-held port means "free it and respawn".
export async function ensureProxyRunning(): Promise<boolean> {
    if (await isProxyReady()) return true;

    const attempts: string[] = [];
    for (let attempt = 0; attempt < 3; attempt++) {
        stopProxy();
        await freeStaleProxyPorts();
        spawnProxy();
        if (await waitForHealthy(4000)) {
            logInfo("proxy_healthy_after_retry");
            return true;
        }
        attempts.push(`attempt=${attempt + 1}: ${await describeProxyState()}`);
        logWarn(`proxy_unhealthy ${attempts[attempts.length - 1]}`);
    }

    lastDiagnostics = attempts.join(" | ");
    console.error(`[GFW Proxy] Failed to become healthy: ${lastDiagnostics}`);
    logError(`proxy_start_failed ${lastDiagnostics}`);
    return false;
}

// Best-effort snapshot of why the proxy is not ready, so the failure dialog and
// connection log carry the actual reason (spawn error, immediate exit, control
// port not answering, or a PID/port mismatch) instead of a generic message.
async function describeProxyState(): Promise<string> {
    const bits: string[] = [];
    if (lastSpawnError) {
        bits.push(`spawnError=${lastSpawnError}`);
        lastSpawnError = null;
    }
    if (lastChildExit) {
        bits.push(`exit=${lastChildExit}`);
        lastChildExit = null;
    }
    if (proxyProcess) {
        bits.push(`childPid=${proxyProcess.pid ?? "?"}`);
        if (proxyProcess.exitCode !== null) bits.push(`childExitCode=${proxyProcess.exitCode}`);
    } else {
        bits.push("child=gone");
    }
    bits.push(`healthy=${await isProxyHealthy()}`);
    bits.push(`servedByUs=${await isProxyServedByUs()}`);
    bits.push(`ctrlConfirmed=${controlBindConfirmedBy === proxyProcess}`);
    bits.push(`controlPortPids=[${(await findPidsOnPort(CONTROL_PORT)).join(",")}]`);
    bits.push(`proxyPortPids=[${(await findPidsOnPort(PROXY_PORT)).join(",")}]`);
    return bits.join(" ");
}

// The app lives for hours and the proxy can die at any moment (network drops,
// OOM, the stale-process scenario above, ...). Without recovery this used to
// mean a dead Discord that needed a manual app restart. The monitor periodically
// checks our proxy and, when it is gone or unhealthy, frees the ports, spawns a
// fresh process and waits for it to become healthy again.
let monitorTimer: NodeJS.Timeout | null = null;
let shuttingDown = false;
let recovering = false;

export function startProxyMonitor(intervalMs = 10_000) {
    if (monitorTimer) return;
    monitorTimer = setInterval(() => {
        void checkAndRecoverProxy();
    }, intervalMs);
    monitorTimer.unref?.();
}

// Called before the app exits so the monitor never respawns the proxy mid-quit.
export function markShuttingDown() {
    shuttingDown = true;
    if (monitorTimer) {
        clearInterval(monitorTimer);
        monitorTimer = null;
    }
}

async function checkAndRecoverProxy() {
    if (recovering || shuttingDown) return;

    const childAlive = proxyProcess !== null && proxyProcess.exitCode === null;
    if (childAlive) {
        if (await isProxyReady()) return;
        // Give a still-booting proxy a moment before declaring it dead.
        if (await waitForHealthy(3000)) return;
    }

    logWarn("proxy_unhealthy_runtime recovering");
    recovering = true;
    try {
        stopProxy();
        await freeStaleProxyPorts();
        spawnProxy();
        if (await waitForHealthy(4000)) {
            console.log("[GFW Proxy] Recovered");
            logInfo("proxy_recovered");
        } else {
            logWarn("proxy_recovery_failed");
        }
    } finally {
        recovering = false;
    }
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

// The control port may answer /status even when the listener is a stale
// orphan, not our own freshly spawned child. Only count the proxy as ready
// when the control port is actually served by the process we spawned.
async function isProxyServedByUs(): Promise<boolean> {
    if (!proxyProcess || proxyProcess.exitCode !== null || proxyProcess.pid === undefined) return false;
    const pids = await findPidsOnPort(CONTROL_PORT);
    return pids.includes(proxyProcess.pid);
}

async function isProxyReady(): Promise<boolean> {
    if (!(await isProxyHealthy())) return false;
    // Primary ownership signal: our own child printed "[CTRL] control server
    // listening", which the Rust proxy emits only after binding the control
    // port. Cheap and works everywhere, including Windows where the netstat
    // PID lookup below is unreliable (it used to report servedByUs=false even
    // when our child was serving /status, causing a working proxy to be killed
    // and the app to hard-fail on startup).
    if (controlBindConfirmedBy === proxyProcess && proxyProcess !== null && proxyProcess.exitCode === null) {
        return true;
    }
    // Fallback: match the control-port listener PID against our spawned child.
    return isProxyServedByUs();
}

function waitForHealthy(timeoutMs: number): Promise<boolean> {
    return new Promise(resolve => {
        const startedAt = Date.now();
        const check = async () => {
            if (await isProxyReady()) return resolve(true);
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

    if (process.platform === "win32") {
        // Windows: no ss/fuser/bash; netstat + taskkill instead.
        // taskkill /T also kills child processes if the stale proxy respawned.
        const pids = [...(await findPidsOnPort(PROXY_PORT)), ...(await findPidsOnPort(CONTROL_PORT))];
        if (pids.length) {
            try {
                await new Promise<void>(resolve => {
                    execFile("taskkill", ["/F", "/T", "/PID", ...pids.map(String)], () => resolve());
                });
            } catch {
                // best effort
            }
        }
        return;
    }

    // POSIX: fuser handles the process-group juggling for us.
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

async function findPidsOnPort(port: number): Promise<number[]> {
    if (process.platform === "win32") {
        return new Promise(resolve => {
            execFile("netstat", ["-ano"], (err, stdout) => {
                if (err) return resolve([]);

                const pids: number[] = [];
                const portToken = new RegExp(`:${port}(\\s|$)`);
                for (const line of stdout.split("\n")) {
                    if (!portToken.test(line)) continue;
                    const pid = line.trim().split(/\s+/).pop();
                    if (pid && /^\d+$/.test(pid)) pids.push(Number(pid));
                }
                resolve(pids.filter(n => Number.isFinite(n) && n > 0));
            });
        });
    }

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

// True only when the USER passed a custom proxy on the command line. This is
// separate from getCustomProxyAddress() because init() appends the built-in
// proxy via app.commandLine.appendSwitch("proxy-server", ...), which makes the
// switch value non-empty even when no custom proxy was requested. Relying on
// getCustomProxyAddress() there would skip ensureProxyRunning() entirely and
// leave a dead/stale proxy undetected until a manual restart.
export function hasCustomProxyOverride(): boolean {
    const idx = process.argv.indexOf("--proxy-server");
    return idx !== -1 && idx + 1 < process.argv.length;
}
