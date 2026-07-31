/*
 * Vegcord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2025 Vendicated and Vegcord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import { app } from "electron";
import { join } from "path";
import { STATIC_DIR } from "shared/paths";

let libVegcord: typeof import("libvegord") | null = null;

function loadLibVegcord() {
    try {
        if (!libVegcord) {
            libVegcord = require(join(STATIC_DIR, `dist/libvegord-${process.arch}.node`));
        }
    } catch (e) {
        console.error("Failed to load libvegord:", e);
    }

    return libVegcord;
}

export function getAccentColor() {
    return loadLibVegcord()?.getAccentColor() ?? null;
}

export function updateUnityLauncherCount(count: number) {
    const libVegcord = loadLibVegcord();
    if (!libVegcord) {
        return app.setBadgeCount(count);
    }

    return libVegcord.updateUnityLauncherCount(count);
}

export function requestBackground(autoStart: boolean, commandLine: string[]) {
    return loadLibVegcord()?.requestBackground(autoStart, commandLine) ?? false;
}
