/*
 * vegord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2023 Vendicated and vegord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import type { BrowserWindowConstructorOptions } from "electron";

export const SplashProps: BrowserWindowConstructorOptions = {
    transparent: true,
    frame: false,
    height: 280,
    width: 280,
    center: true,
    resizable: false,
    maximizable: false,
    alwaysOnTop: true,
    backgroundColor: "#00000000"
};
