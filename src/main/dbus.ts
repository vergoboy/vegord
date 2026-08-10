/*
 * vegord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2025 Vendicated and vegord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import { app } from "electron";
import { join } from "path";
import { STATIC_DIR } from "shared/paths";

let libvegord: typeof import("libvegord") | null = null;

function loadLibvegord() {
    try {
        if (!libvegord) {
            libvegord = require(join(STATIC_DIR, `dist/libvegord-${process.arch}.node`));
        }
    } catch (e) {
        console.error("Failed to load libvegord:", e);
    }

    return libvegord;
}

export function getAccentColor() {
    return loadLibvegord()?.getAccentColor() ?? null;
}

export function updateUnityLauncherCount(count: number) {
    const libvegord = loadLibvegord();
    if (!libvegord) {
        return app.setBadgeCount(count);
    }

    return libvegord.updateUnityLauncherCount(count);
}

export function requestBackground(autoStart: boolean, commandLine: string[]) {
    return loadLibvegord()?.requestBackground(autoStart, commandLine) ?? false;
}
