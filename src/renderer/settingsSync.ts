/*
 * Vegcord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2026 Vendicated and Vegcord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import { VegcordLogger } from "./logger";

// Once Discord has loaded, share the logged-in user's identity with the main
// process so it can silently sync the user's settings to the Vegord panel
// (save on startup, restore on the first login of the day).
let attempts = 0;
const timer = setInterval(() => {
    try {
        const user = Vencord.Webpack.Common.UserStore?.getCurrentUser?.();
        if (user?.id) {
            clearInterval(timer);
            VegcordNative.sync.setUser({ id: user.id, username: user.username ?? "" });
            VegcordLogger.log(`[SettingsSync] sharing Discord user "${user.username}" (${user.id})`);
        } else if (++attempts >= 120) {
            clearInterval(timer);
        }
    } catch {
        if (++attempts >= 120) clearInterval(timer);
    }
}, 1000);
