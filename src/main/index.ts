/*
 * Vegcord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2026 Vendicated and Vegcord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import { app } from "electron";

import { CommandLine } from "./cli";
import { downloadVencordFiles } from "./utils/vencordLoader";

if (CommandLine.values.repair) {
    console.log("Repairing Vegcord...");
    downloadVencordFiles().then(() => app.quit());
} else {
    require("./main");
}
