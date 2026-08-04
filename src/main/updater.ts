/*
 * Vegcord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2025 Vendicated and Vegcord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import { autoUpdater } from "electron-updater";
import { IpcEvents } from "shared/IpcEvents";

import { handle } from "./utils/ipcWrappers";

// Fully automatic updates: check on startup, silently download the update in
// the background, and install it when the app quits. No notification popup and
// nothing optional.
autoUpdater.autoDownload = true;
autoUpdater.autoInstallOnAppQuit = true;
autoUpdater.fullChangelog = false;

autoUpdater.on("checking-for-update", () => console.log("[Updater] Checking for update"));
autoUpdater.on("update-available", info => console.log("[Updater] Update available:", info.version));
autoUpdater.on("update-not-available", () => console.log("[Updater] No update available"));
autoUpdater.on("update-downloaded", info => console.log("[Updater] Update downloaded:", info.version));
autoUpdater.on("error", err => console.error("[Updater] Error:", err.message));

// Keep the renderer IPC surface inert: updates are automatic now, so the
// settings "outdated" warning must never appear and there is nothing to open.
handle(IpcEvents.UPDATER_IS_OUTDATED, () => false);
handle(IpcEvents.UPDATER_OPEN, () => void 0);

autoUpdater.checkForUpdates().catch(err => console.error("[Updater] Update check failed:", err.message));
