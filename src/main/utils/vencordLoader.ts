/*
 * Vegcord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2023 Vendicated and Vencord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import { existsSync, mkdirSync } from "fs";
import { copyFile, writeFile } from "fs/promises";
import { VENCORD_FILES_DIR } from "main/vencordFilesDir";
import { join } from "path";

import { USER_AGENT } from "../constants";
import { downloadFile, fetchie } from "./http";

const API_BASE = "https://api.github.com";
const BUNDLED_VENCORD_DIR = join(__dirname, "..", "..", "static", "vencordFiles");

export const FILES_TO_DOWNLOAD = [
    "vencordDesktopMain.js",
    "vencordDesktopPreload.js",
    "vencordDesktopRenderer.js",
    "vencordDesktopRenderer.css"
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

export async function downloadVencordFiles() {
    const release = await githubGet("/repos/Vendicated/Vencord/releases/latest");

    const { assets }: ReleaseData = await release.json();

    await Promise.all(
        assets
            .filter(({ name }) => FILES_TO_DOWNLOAD.some(f => name.startsWith(f)))
            .map(({ name, browser_download_url }) =>
                downloadFile(browser_download_url, join(VENCORD_FILES_DIR, name), {}, { retryOnNetworkError: true })
            )
    );
}

// NOTE: must use existsSync, not fs.promises.access(F_OK): in the packaged
// app the bundled Vencord files live inside app.asar, and Electron's asar
// support fails ENOENT for access() on asar directories (works for files and
// for existsSync). Using access() here silently disabled the bundled vegord
// Vencord build on Windows and fell back to downloading stock Vencord.
const existsAsync = (path: string) => Promise.resolve(existsSync(path));

export async function isValidVencordInstall(dir: string) {
    const results = await Promise.all(["package.json", ...FILES_TO_DOWNLOAD].map(f => existsAsync(join(dir, f))));
    return !results.includes(false);
}

export async function copyBundledVencordFiles() {
    if (!(await existsAsync(BUNDLED_VENCORD_DIR))) return false;

    await Promise.all(FILES_TO_DOWNLOAD.map(f => copyFile(join(BUNDLED_VENCORD_DIR, f), join(VENCORD_FILES_DIR, f))));
    return true;
}

export async function ensureVencordFiles(force = false) {
    mkdirSync(VENCORD_FILES_DIR, { recursive: true });

    if (!(await copyBundledVencordFiles())) {
        await Promise.all([downloadVencordFiles(), writeFile(join(VENCORD_FILES_DIR, "package.json"), "{}")]);
    } else {
        await writeFile(join(VENCORD_FILES_DIR, "package.json"), "{}");
    }
}
