/*
 * Vegcord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2023 Vendicated and Vencord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import { BrowserWindow, ipcMain, nativeImage, shell } from "electron";
import { existsSync } from "fs";
import { join } from "path";
import { SplashProps } from "shared/browserWinProperties";

import { ICON_PATH } from "./constants";
import { Settings } from "./settings";
import { loadView } from "./vegordStatic";

ipcMain.on("open-external", (_event, url: string) => {
    shell.openExternal(url).catch((err: Error) => console.error("Failed to open URL:", err));
});

let splash: BrowserWindow | undefined;

export function createSplashWindow(startMinimized = false) {
    splash = new BrowserWindow({
        ...SplashProps,
        show: !startMinimized,
        transparent: true,
        backgroundColor: "#00000000",
        webPreferences: {
            preload: join(__dirname, "splashPreload.js")
        }
    });

    if (existsSync(ICON_PATH)) {
        splash.setIcon(nativeImage.createFromPath(ICON_PATH));
    }

    loadView(splash, "splash.html");

    const { splashBackground, splashColor, splashTheming, splashPixelated } = Settings.store;

    if (splashTheming !== false) {
        if (splashColor) {
            const semiTransparentSplashColor = splashColor.replace("rgb(", "rgba(").replace(")", ", 0.2)");

            splash.webContents.insertCSS(`body { --fg: ${splashColor} !important }`);
            splash.webContents.insertCSS(`body { --fg-semi-trans: ${semiTransparentSplashColor} !important }`);
        }

        if (splashBackground) {
            splash.webContents.insertCSS(`body { --bg: ${splashBackground} !important }`);
        }
    }

    if (splashPixelated) {
        splash.webContents.insertCSS(`img { image-rendering: pixelated; }`);
    }

    return splash;
}

export function updateSplashMessage(message: string) {
    if (splash && !splash.isDestroyed()) splash.webContents.send("update-splash-message", message);
}
