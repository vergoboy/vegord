/*
 * vegord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2026 Vendicated and vegord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import { app } from "electron";

import { CommandLine } from "./cli";
import { ensureVegordFiles } from "./utils/vegordLoader";

if (CommandLine.values.repair) {
    console.log("Repairing vegord...");
    ensureVegordFiles(true).then(() => app.quit());
} else {
    require("./main");
}
