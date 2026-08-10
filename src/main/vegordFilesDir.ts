/*
 * vegord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2025 Vendicated and vegord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import { join } from "path";

import { SESSION_DATA_DIR } from "./constants";
import { State } from "./settings";

// this is in a separate file to avoid circular dependencies
export const VEGORD_FILES_DIR = State.store.vegordDir || join(SESSION_DATA_DIR, "vegordFiles");
