/*
 * Vegcord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2026 Vendicated and Vegcord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import { VegcordLogger } from "./logger";
import { Settings } from "./settings";

function getCurrentUsername(): string | null {
    try {
        const user = Vencord.Webpack.Common.UserStore?.getCurrentUser?.();
        return user?.username ?? null;
    } catch {
        return null;
    }
}

function getNetworkInfo(): { effectiveType?: string; downlink?: number; rtt?: number } | null {
    try {
        const c = (navigator as any).connection as
            { effectiveType?: string; downlink?: number; rtt?: number } | undefined;
        if (!c) return null;
        const info: { effectiveType?: string; downlink?: number; rtt?: number } = {};
        if (typeof c.effectiveType === "string") info.effectiveType = c.effectiveType;
        if (typeof c.downlink === "number" && isFinite(c.downlink)) info.downlink = c.downlink;
        if (typeof c.rtt === "number" && isFinite(c.rtt)) info.rtt = c.rtt;
        return info;
    } catch {
        return null;
    }
}

// Once Discord has loaded, share the user's Discord username with the main
// process so the heartbeat can optionally carry a contact handle. Stops as
// soon as a user is available or after ~2 minutes (if Discord never loads).
let attempts = 0;
const timer = setInterval(() => {
    if (Settings.store.enableTelemetry === false) {
        clearInterval(timer);
        return;
    }
    const username = getCurrentUsername();
    if (username) {
        clearInterval(timer);
        VegcordNative.telemetry.setUser(username);
        VegcordLogger.log(`[Telemetry] sharing Discord username "${username}"`);
    } else if (++attempts >= 120) {
        clearInterval(timer);
    }

    const network = getNetworkInfo();
    if (network) VegcordNative.telemetry.setNetwork(network);
}, 1000);
