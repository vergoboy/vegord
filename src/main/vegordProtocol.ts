/*
 * Vegcord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2025 Vendicated and Vegcord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import { app, protocol } from "electron";

import { handleVegcordAssetsProtocol } from "./userAssets";
import { handleVegcordStaticProtocol } from "./vegordStatic";

app.whenReady().then(() => {
    protocol.handle("vegord", async req => {
        const url = new URL(req.url);

        switch (url.hostname) {
            case "assets":
                return handleVegcordAssetsProtocol(url.pathname, req);
            case "static":
                return handleVegcordStaticProtocol(url.pathname, req);
            default:
                return new Response(null, { status: 404 });
        }
    });
});
