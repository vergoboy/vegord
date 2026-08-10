/*
 * vegord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2023 Vendicated and vegord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

declare global {
    export var vegordNative: typeof import("preload/vegordNative").vegordNative;
    export var vegord: typeof import("@vencord/types/Vencord");
    export var vegordMod: typeof import("@vencord/types/VencordNative").default;
    export var vegordApp: typeof import("renderer/index");
    export var vegordPatchGlobals: any;

    export var IS_DEV: boolean;
}

export {};
