/*
 * vegord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2026 Vendicated and vegord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import { vegordLogger } from "./logger";

// Once Discord has loaded, share the logged-in user's identity with the main
// process so it can silently sync the user's settings to the Vegord panel
// (save on startup, restore on the first login of the day).
let attempts = 0;
const timer = setInterval(() => {
    try {
        const user = vegord.Webpack.Common.UserStore?.getCurrentUser?.();
        if (user?.id) {
            clearInterval(timer);
            vegordNative.sync.setUser({ id: user.id, username: user.username ?? "" });
            vegordLogger.log(`[SettingsSync] sharing Discord user "${user.username}" (${user.id})`);
        } else if (++attempts >= 120) {
            clearInterval(timer);
        }
    } catch {
        if (++attempts >= 120) clearInterval(timer);
    }
}, 1000);
