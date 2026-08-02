/*
 * Vegcord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2023 Vendicated and Vencord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import "./cli";
import "./updater";
import "./ipc";
import "./userAssets";
import "./vegordProtocol";

import { existsSync } from "fs";
import { join } from "path";

import { app, BrowserWindow, dialog, nativeTheme } from "electron";

import { startAnnouncements } from "./announcements";
import { flushBeforeQuit, logInfo, startConnectionLog } from "./connectionLog";
import { DATA_DIR } from "./constants";
import { logProxyStatus } from "./dohControl";
import { createFirstLaunchTour } from "./firstLaunch";
import { ensureProxyRunning, getCustomProxyAddress, getProxyAddress, startProxy, stopProxy } from "./gfwProxy";
import { createWindows, mainWin } from "./mainWindow";
import { registerMediaPermissionsHandler } from "./mediaPermissions";
import { registerScreenShareHandler } from "./screenShare";
import { Settings, State } from "./settings";
import { startGithubUpdateChecker, startTelemetry } from "./telemetry";
import { setAsDefaultProtocolClient } from "./utils/setAsDefaultProtocolClient";
import { isDeckGameMode } from "./utils/steamOS";

console.log("Vegcord v" + app.getVersion());
logInfo("app_start");

// Make the Vencord files use our DATA_DIR
process.env.VENCORD_USER_DATA_DIR = DATA_DIR;

const isLinux = process.platform === "linux";
const isWindows = process.platform === "win32";

export let enableHardwareAcceleration = true;

function init() {
    setAsDefaultProtocolClient("discord");

    const { disableSmoothScroll, hardwareAcceleration, hardwareVideoAcceleration } = Settings.store;

    const enabledFeatures = new Set(app.commandLine.getSwitchValue("enable-features").split(","));
    const disabledFeatures = new Set(app.commandLine.getSwitchValue("disable-features").split(","));
    app.commandLine.removeSwitch("enable-features");
    app.commandLine.removeSwitch("disable-features");

    if (hardwareAcceleration === false || process.argv.includes("--disable-gpu")) {
        enableHardwareAcceleration = false;
        app.disableHardwareAcceleration();
    } else {
        if (hardwareVideoAcceleration) {
            enabledFeatures.add("AcceleratedVideoEncoder");
            enabledFeatures.add("AcceleratedVideoDecoder");

            if (isLinux) {
                enabledFeatures.add("AcceleratedVideoDecodeLinuxGL");
                enabledFeatures.add("AcceleratedVideoDecodeLinuxZeroCopyGL");
            }
        }
    }

    if (disableSmoothScroll) {
        app.commandLine.appendSwitch("disable-smooth-scrolling");
    }

    // disable renderer backgrounding to prevent the app from unloading when in the background
    // https://github.com/electron/electron/issues/2822
    // https://github.com/GoogleChrome/chrome-launcher/blob/5a27dd574d47a75fec0fb50f7b774ebf8a9791ba/docs/chrome-flags-for-tools.md#task-throttling
    app.commandLine.appendSwitch("disable-renderer-backgrounding");
    app.commandLine.appendSwitch("disable-background-timer-throttling");
    app.commandLine.appendSwitch("disable-backgrounding-occluded-windows");
    if (process.platform === "win32") {
        disabledFeatures.add("CalculateNativeWinOcclusion");
    }

    // work around chrome 66 disabling autoplay by default
    app.commandLine.appendSwitch("autoplay-policy", "no-user-gesture-required");

    // WinRetrieveSuggestionsOnlyOnDemand: Work around electron 13 bug w/ async spellchecking on Windows.
    // HardwareMediaKeyHandling, MediaSessionService: Prevent Discord from registering as a media service.
    disabledFeatures.add("WinRetrieveSuggestionsOnlyOnDemand");
    disabledFeatures.add("HardwareMediaKeyHandling");
    disabledFeatures.add("MediaSessionService");

    if (isLinux || isWindows) {
        // Use the GFW-resistant proxy (or a custom --proxy-server passed on the CLI).
        const customProxy = getCustomProxyAddress();
        if (customProxy) {
            console.log(`[Proxy] Using custom proxy: ${customProxy}`);
        } else {
            app.commandLine.appendSwitch("proxy-server", getProxyAddress());
            console.log(`[Proxy] Using proxy: ${getProxyAddress()}`);
        }

        // Force all WebRTC traffic through proxy (critical for voice to work over SOCKS5)
        app.commandLine.appendSwitch("webrtc-ip-handling-policy", "disable_non_proxied_udp");
        console.log("[Voice] WebRTC forced through proxy (disable_non_proxied_udp)");

        // Support TTS on Linux using https://wiki.archlinux.org/title/Speech_dispatcher
        // Only enable when the speechd daemon is actually installed: without it,
        // Chromium's TTS engine calls spd_open(), which tries to spawn the `speechd`
        // binary and crashes the browser process with SIGTRAP on startup.
        if (isLinux && (process.env.PATH ?? "").split(":").some(dir => existsSync(join(dir, "speechd")))) {
            app.commandLine.appendSwitch("enable-speech-dispatcher");
        }

        // This is needed to fix washed out colours - https://github.com/electron/electron/issues/49566
        // Supposed to be fixed already according to comments there, but it's just not lol, I can repro on Electron 43.0.0
        // when moving the window from my main monitor (HDR - not sure if this is relevant lol) to second monitor (SDR) and back
        if (isLinux) {
            disabledFeatures.add("WaylandWpColorManagerV1");
        }

        // Log voice-relevant Chrome flags for debugging
        console.log("[Voice] Proxy SOCKS5=127.0.0.1:4500, WebRTC=disable_non_proxied_udp");
    }

    disabledFeatures.forEach(feat => enabledFeatures.delete(feat));

    const enabledFeaturesArray = enabledFeatures.values().filter(Boolean).toArray();
    const disabledFeaturesArray = disabledFeatures.values().filter(Boolean).toArray();

    if (enabledFeaturesArray.length) {
        app.commandLine.appendSwitch("enable-features", enabledFeaturesArray.join(","));
        console.log("Enabled Chromium features:", enabledFeaturesArray.join(", "));
    }

    if (disabledFeaturesArray.length) {
        app.commandLine.appendSwitch("disable-features", disabledFeaturesArray.join(","));
        console.log("Disabled Chromium features:", disabledFeaturesArray.join(", "));
    }

    // Start the GFW-resistant proxy in the background. The app cannot run
    // without it: bootstrap() verifies it is healthy before Discord loads
    // and quits with an error if it cannot be started.
    startProxy();

    // In the Flatpak on SteamOS the theme is detected as light, but SteamOS only has a dark mode, so we just override it
    if (isDeckGameMode) nativeTheme.themeSource = "dark";

    app.on("second-instance", (_event, _cmdLine, _cwd, data: any) => {
        if (data.IS_DEV) app.quit();
        else if (mainWin) {
            if (mainWin.isMinimized()) mainWin.restore();
            if (!mainWin.isVisible()) mainWin.show();
            mainWin.focus();
        } else {
            // The first instance is alive but has no window (still booting, or
            // a leftover/tray instance). Surface the app instead of silently
            // doing nothing, so relaunching always brings a window up.
            createWindows();
        }
    });

    app.whenReady().then(async () => {
        if (process.platform === "win32") app.setAppUserModelId("dev.vencord.vegord");
        if (process.platform === "linux") {
            try {
                app.setAppUserModelId("vegord");
            } catch {}

            // Match the .desktop file's StartupWMClass so the taskbar/titlebar
            // shows the Vegcord icon instead of Electron's default one
            try {
                app.setDesktopName("vegord-gfw.desktop");
            } catch {}
        }

        registerScreenShareHandler();
        registerMediaPermissionsHandler();

        bootstrap();

        app.on("activate", () => {
            if (BrowserWindow.getAllWindows().length === 0) createWindows();
        });
    });
}

if (!app.requestSingleInstanceLock({ IS_DEV })) {
    if (IS_DEV) {
        console.log("Vegcord is already running. Quitting previous instance...");
        init();
    } else {
        console.log(
            "Vegcord is already running (another instance holds the app lock). Quitting...\n" +
                "If no Vegcord window appears, an instance is likely still running in the system tray.\n" +
                "Close it via the tray icon, or run: taskkill /F /IM vegord.exe"
        );
        app.quit();
    }
} else {
    init();
}

// Client telemetry (heartbeat), connection log uploads, GitHub-API update checker
startTelemetry();
startConnectionLog();
startGithubUpdateChecker();

// Periodic DoH status snapshots (every 15 min) so the panel records which
// server each network/ISP ends up on over time.
setInterval(() => logProxyStatus("periodic"), 15 * 60 * 1000);

// Persistent panel announcements (visible until the user dismisses them)
startAnnouncements();

async function bootstrap() {
    // The app must never run without a working proxy. When no custom proxy is
    // given on the CLI, wait for our proxy to answer and hard-fail if it
    // cannot be started (e.g. a leftover process holding the ports).
    if (!getCustomProxyAddress() && !(await ensureProxyRunning())) {
        console.error("Vegcord could not start its network proxy. Quitting because the app requires it.");
        dialog.showErrorBox(
            "Vegcord requires the GFW proxy",
            "Vegcord could not start its network proxy, so the app cannot run.\n\n" +
                "Please make sure no leftover Vegcord process is running and try again.\n\n" +
                "If the error persists, delete ~/.config/vegord/proxy and restart."
        );
        app.exit(1);
        return;
    }

    if (!Object.hasOwn(State.store, "firstLaunch")) {
        createFirstLaunchTour();
    } else {
        createWindows();
    }
}

// MacOS only event
export let darwinURL: string | undefined;
app.on("open-url", (_, url) => {
    darwinURL = url;
});

app.on("window-all-closed", () => {
    if (process.platform !== "darwin") {
        stopProxy();
        app.quit();
    }
});

app.on("before-quit", () => {
    flushBeforeQuit();
    stopProxy();
});

// Sets the WebRTC IP handling policy for all current and future windows.
// Switching to "default_public_and_private_interfaces" may fix calls stuck at "DTLS Connecting" when using VPNs, Tailscale, etc.
// https://github.com/Vencord/Vegcord/issues/876
app.on("web-contents-created", (_event, contents) => {
    contents.setWebRTCIPHandlingPolicy(Settings.store.webRTCIPHandlingPolicy ?? "default");
});
Settings.addChangeListener("webRTCIPHandlingPolicy", () => {
    for (const win of BrowserWindow.getAllWindows()) {
        win.webContents.setWebRTCIPHandlingPolicy(Settings.store.webRTCIPHandlingPolicy ?? "default");
    }
});
