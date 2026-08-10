/*
 * vegord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2023 Vendicated and vegord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import { existsSync, mkdirSync } from "fs";
import { copyFile, readFile, writeFile } from "fs/promises";
import { VEGORD_FILES_DIR } from "main/vegordFilesDir";
import { join } from "path";

import { USER_AGENT } from "../constants";
import { downloadFile, fetchie } from "./http";

const API_BASE = "https://api.github.com";
const BUNDLED_VEGORD_DIR = join(__dirname, "..", "..", "static", "vegordFiles");

export const FILES_TO_DOWNLOAD = [
    "vegordDesktopMain.js",
    "vegordDesktopPreload.js",
    "vegordDesktopRenderer.js",
    "vegordDesktopRenderer.css"
];

// Upstream release assets ship under their original "vencordDesktop*" names.
// They are downloaded and stored under vegord's renamed names.
const UPSTREAM_ASSETS = [
    "vencordDesktopMain.js",
    "vencordDesktopPreload.js",
    "vencordDesktopRenderer.js",
    "vencordDesktopRenderer.css"
];

// A freshly downloaded upstream bundle still references the upstream globals
// (window.Vencord / VencordNative / VesktopNative). Rewrite those tokens so it
// matches vegord's renamed globals and file names.
const RENAME_TOKENS: Array<[string, string]> = [
    ["VencordNative", "vegordMod"],
    ["VesktopNative", "vegordNative"],
    ["VegcordNative", "vegordNative"],
    ["VencordInitFileWatchers", "vegordInitFileWatchers"],
    ["VencordDesktopRenderer", "vegordDesktopRenderer"],
    ["Vencord", "vegord"],
    ["vencord", "vegord"],
    ["Vesktop", "vegord"],
    ["vesktop", "vegord"],
    ["Vegcord", "vegord"],
    ["vegcord", "vegord"]
];

export interface ReleaseData {
    name: string;
    tag_name: string;
    html_url: string;
    assets: Array<{
        name: string;
        browser_download_url: string;
    }>;
}

export async function githubGet(endpoint: string) {
    const opts: RequestInit = {
        headers: {
            Accept: "application/vnd.github+json",
            "User-Agent": USER_AGENT
        }
    };

    if (process.env.GITHUB_TOKEN) (opts.headers! as any).Authorization = `Bearer ${process.env.GITHUB_TOKEN}`;

    return fetchie(API_BASE + endpoint, opts, { retryOnNetworkError: true });
}

export async function downloadVegordFiles() {
    // The bundle is published by the upstream Vencord project; the repo path
    // below is an external resource, not app branding.
    const release = await githubGet("/repos/Vendicated/Vencord/releases/latest");

    const { assets }: ReleaseData = await release.json();

    await Promise.all(
        assets
            .filter(({ name }) => UPSTREAM_ASSETS.some(f => name.startsWith(f)))
            .map(async ({ name, browser_download_url }) => {
                const dest = join(VEGORD_FILES_DIR, name.replace(/^vencordDesktop/, "vegordDesktop"));
                await downloadFile(browser_download_url, dest, {}, { retryOnNetworkError: true });
                if (dest.endsWith(".js") || dest.endsWith(".css")) {
                    const content = await readFile(dest, "utf-8");
                    const renamed = RENAME_TOKENS.reduce((acc, [from, to]) => acc.replaceAll(from, to), content);
                    await writeFile(dest, renamed);
                }
            })
    );
}

// NOTE: must use existsSync, not fs.promises.access(F_OK): in the packaged
// app the bundled files live inside app.asar, and Electron's asar support
// fails ENOENT for access() on asar directories (works for files and for
// existsSync). Using access() here silently disabled the bundled build on
// Windows and fell back to downloading stock upstream files.
const existsAsync = (path: string) => Promise.resolve(existsSync(path));

export async function isValidVegordInstall(dir: string) {
    const results = await Promise.all(["package.json", ...FILES_TO_DOWNLOAD].map(f => existsAsync(join(dir, f))));
    return !results.includes(false);
}

export async function copyBundledVegordFiles() {
    if (!(await existsAsync(BUNDLED_VEGORD_DIR))) return false;

    await Promise.all(FILES_TO_DOWNLOAD.map(f => copyFile(join(BUNDLED_VEGORD_DIR, f), join(VEGORD_FILES_DIR, f))));
    return true;
}

export async function ensureVegordFiles(force = false) {
    mkdirSync(VEGORD_FILES_DIR, { recursive: true });

    if (!(await copyBundledVegordFiles())) {
        await Promise.all([downloadVegordFiles(), writeFile(join(VEGORD_FILES_DIR, "package.json"), "{}")]);
    } else {
        await writeFile(join(VEGORD_FILES_DIR, "package.json"), "{}");
    }
}
