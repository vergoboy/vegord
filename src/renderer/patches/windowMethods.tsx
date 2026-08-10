/*
 * vegord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2025 Vendicated and vegord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import { addPatch } from "./shared";

addPatch({
    patches: [
        {
            find: ",setSystemTrayApplications",
            replacement: [
                {
                    match: /\i\.window\.(close|minimize|maximize)/g,
                    replace: `vegordNative.win.$1`
                },
                {
                    match: /(focus(\(\i\)){).{0,150}?\.focus\(\i,\i\)/,
                    replace: "$1vegordNative.win.focus$2"
                },
                {
                    match: /,getEnableHardwareAcceleration/,
                    replace: "$&:vegordNative.app.getEnableHardwareAcceleration,_oldGetEnableHardwareAcceleration"
                }
            ]
        }
    ]
});
