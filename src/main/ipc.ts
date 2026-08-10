/*
 * vegord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2023 Vendicated and vegord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

if (process.platform === "linux") import("./venmic");

import { execFile } from "child_process";
import {
    app,
    BrowserWindow,
    clipboard,
    dialog,
    IpcMainInvokeEvent,
    nativeImage,
    RelaunchOptions,
    session,
    shell
} from "electron";
import { readFileSync, watch } from "fs";
import { readFile, stat } from "fs/promises";
import { release } from "os";
import { join } from "path";

import { IpcEvents } from "../shared/IpcEvents";
import { setBadgeCount } from "./appBadge";
import { autoStart } from "./autoStart";
import { enableHardwareAcceleration } from "./main";
import { mainWin } from "./mainWindow";
import { Settings, State } from "./settings";
import { requestSettingsSync } from "./settingsSync";
import { handle, handleSync } from "./utils/ipcWrappers";
import { PopoutWindows } from "./utils/popout";
import { isDeckGameMode, showGamePage } from "./utils/steamOS";
import { isValidVegordInstall } from "./utils/vegordLoader";
import { VEGORD_FILES_DIR } from "./vegordFilesDir";

handleSync(IpcEvents.GET_VEGORD_MOD_PRELOAD_SCRIPT, () =>
    readFileSync(join(VEGORD_FILES_DIR, "vegordDesktopPreload.js"), "utf-8")
);
handleSync(IpcEvents.GET_VEGORD_MOD_RENDERER_SCRIPT, () =>
    readFileSync(join(VEGORD_FILES_DIR, "vegordDesktopRenderer.js"), "utf-8")
);

const VEGORD_RENDERER_JS_PATH = join(__dirname, "renderer.js");
const VEGORD_RENDERER_CSS_PATH = join(__dirname, "renderer.css");
handleSync(IpcEvents.GET_VEGORD_RENDERER_SCRIPT, () => readFileSync(VEGORD_RENDERER_JS_PATH, "utf-8"));
handle(IpcEvents.GET_VEGORD_RENDERER_CSS, () => readFile(VEGORD_RENDERER_CSS_PATH, "utf-8"));

if (IS_DEV) {
    watch(VEGORD_RENDERER_CSS_PATH, { persistent: false }, async () => {
        mainWin?.webContents.postMessage(
            IpcEvents.VEGORD_RENDERER_CSS_UPDATE,
            await readFile(VEGORD_RENDERER_CSS_PATH, "utf-8")
        );
    });
}

handleSync(IpcEvents.GET_SETTINGS, () => Settings.plain);
handleSync(IpcEvents.GET_VERSION, () => app.getVersion());
handleSync(IpcEvents.GET_ENABLE_HARDWARE_ACCELERATION, () => enableHardwareAcceleration);

handleSync(
    IpcEvents.SUPPORTS_WINDOWS_TRANSPARENCY,
    () => process.platform === "win32" && Number(release().split(".").pop()) >= 22621
);

handleSync(IpcEvents.AUTOSTART_ENABLED, () => autoStart.isEnabled());
handle(IpcEvents.ENABLE_AUTOSTART, autoStart.enable);
handle(IpcEvents.DISABLE_AUTOSTART, autoStart.disable);

handle(IpcEvents.SET_SETTINGS, (_, settings: typeof Settings.store, path?: string) => {
    Settings.setData(settings, path);
    requestSettingsSync();
});

handle(IpcEvents.RELAUNCH, async () => {
    const options: RelaunchOptions = {
        args: process.argv.slice(1).concat(["--relaunch"])
    };
    if (isDeckGameMode) {
        // We can't properly relaunch when running under gamescope, but we can at least navigate to our page in Steam.
        await showGamePage();
    } else if (app.isPackaged && process.env.APPIMAGE) {
        execFile(process.env.APPIMAGE, options.args);
    } else {
        app.relaunch(options);
    }
    app.exit();
});

handleSync(IpcEvents.IS_USING_CUSTOM_VEGORD_DIR, () => !!State.store.vegordDir);
handle(IpcEvents.SHOW_CUSTOM_VEGORD_DIR, async () => {
    const { vegordDir } = State.store;
    if (!vegordDir) return;

    const stats = await stat(vegordDir);
    if (!stats.isDirectory()) return;

    shell.openPath(vegordDir);
});

function getWindow(e: IpcMainInvokeEvent, key?: string) {
    return key ? PopoutWindows.get(key)! : (BrowserWindow.fromWebContents(e.sender) ?? mainWin);
}

handle(IpcEvents.FOCUS, () => {
    mainWin.show();
    mainWin.setSkipTaskbar(false);
});

handle(IpcEvents.CLOSE, (e, key?: string) => {
    getWindow(e, key).close();
});

handle(IpcEvents.MINIMIZE, (e, key?: string) => {
    getWindow(e, key).minimize();
});

handle(IpcEvents.MAXIMIZE, (e, key?: string) => {
    const win = getWindow(e, key);
    if (win.isMaximized()) {
        win.unmaximize();
    } else {
        win.maximize();
    }
});

handleSync(IpcEvents.SPELLCHECK_GET_AVAILABLE_LANGUAGES, e => {
    e.returnValue = session.defaultSession.availableSpellCheckerLanguages;
});

handle(IpcEvents.SPELLCHECK_REPLACE_MISSPELLING, (e, word: string) => {
    e.sender.replaceMisspelling(word);
});

handle(IpcEvents.SPELLCHECK_ADD_TO_DICTIONARY, (e, word: string) => {
    e.sender.session.addWordToSpellCheckerDictionary(word);
});

handle(IpcEvents.SELECT_VEGORD_DIR, async (_e, value?: null) => {
    if (value === null) {
        delete State.store.vegordDir;
        return "ok";
    }

    const res = await dialog.showOpenDialog(mainWin!, {
        properties: ["openDirectory"]
    });
    if (!res.filePaths.length) return "cancelled";

    const dir = res.filePaths[0];
    if (!isValidVegordInstall(dir)) return "invalid";

    State.store.vegordDir = dir;

    return "ok";
});

handle(IpcEvents.SET_BADGE_COUNT, (_, count: number) => setBadgeCount(count));

handle(IpcEvents.FLASH_FRAME, (_, flag: boolean) => {
    if (!mainWin || mainWin.isDestroyed() || (flag && mainWin.isFocused())) return;
    mainWin.flashFrame(flag);
});

handle(IpcEvents.CLIPBOARD_COPY_IMAGE, async (_, buf: ArrayBuffer, src: string) => {
    clipboard.write({
        html: `<img src="${src.replaceAll('"', '\\"')}">`,
        image: nativeImage.createFromBuffer(Buffer.from(buf))
    });
});

function openDebugPage(page: string) {
    const win = new BrowserWindow({
        autoHideMenuBar: true
    });

    win.loadURL(page);
}

handle(IpcEvents.DEBUG_LAUNCH_GPU, () => openDebugPage("chrome://gpu"));
handle(IpcEvents.DEBUG_LAUNCH_WEBRTC_INTERNALS, () => openDebugPage("chrome://webrtc-internals"));
