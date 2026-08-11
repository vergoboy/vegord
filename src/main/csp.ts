/*
 * vegord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2026 Vendicated and vegord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import { session } from "electron";

/**
 * Rewrites the Discord webapp's Content-Security-Policy on the main-frame
 * response so vegord's own injected resources are allowed:
 *
 * - inline `<style>`/`<script>` ("unsafe-inline", "unsafe-eval")
 * - `data:` / `blob:` / `vegord:` sources (the bundled emoji font and the
 *   message-bubble CSS are served from these)
 * - well-known user-content domains (imgur, tenor, GitHub Pages, jsDelivr,
 *   ...) so QuickCSS/theme resources load without manual "Allow" clicks
 *
 * This mirrors the CSP rewrite that the mod's own main bundle performs, but
 * lives in the app's main process where it is guaranteed to run before the
 * mod bundle (and its main-side init) even loads — so a mod-side init crash
 * can never silently resurrect the stock CSP blocks.
 */

const CSP_DIRECTIVES = ["style-src", "connect-src", "img-src", "font-src", "media-src", "worker-src"];

const CONNECT = ["connect-src"];
const IMG = [...CONNECT, "img-src"];
const STYLE_FONT = ["style-src", "font-src"];
const MEDIA = [...IMG, "media-src"];
const ALL = [...IMG, ...STYLE_FONT];
const ALL_SCRIPT = [...ALL, "script-src", "worker-src"];

const DEFAULT_ALLOWED: Record<string, string[]> = {
    "http://localhost:*": ALL,
    "http://127.0.0.1:*": ALL,
    "localhost:*": ALL,
    "127.0.0.1:*": ALL,
    "*.github.io": ALL,
    "github.com": ALL,
    "raw.githubusercontent.com": ALL,
    "*.gitlab.io": ALL,
    "gitlab.com": ALL,
    "*.codeberg.page": ALL,
    "codeberg.org": ALL,
    "*.githack.com": ALL,
    "jsdelivr.net": ALL,
    "fonts.googleapis.com": STYLE_FONT,
    "i.imgur.com": IMG,
    "i.ibb.co": IMG,
    "i.pinimg.com": IMG,
    "files.catbox.moe": ALL,
    "cdn.discordapp.com": ALL,
    "media.discordapp.net": IMG,
    "cdnjs.cloudflare.com": ALL_SCRIPT,
    "cdn.jsdelivr.net": ALL_SCRIPT,
    "api.github.com": CONNECT,
    "ws.audioscrobbler.com": CONNECT,
    "musicbrainz.org": CONNECT,
    "*.listenbrainz.org": CONNECT,
    "coverartarchive.org": CONNECT,
    "archive.org": CONNECT,
    "*.archive.org": CONNECT,
    "translate-pa.googleapis.com": CONNECT,
    "*.vegord.dev": IMG,
    "manti.vendicated.dev": IMG,
    "decor.fieryflames.dev": CONNECT,
    "ugc.decor.fieryflames.dev": IMG,
    "sponsor.ajay.app": CONNECT,
    "dearrow-thumb.ajay.app": IMG,
    "usrbg.is-hardly.online": IMG,
    "icons.duckduckgo.com": IMG,
    "*.tenor.com": MEDIA,
    "*.tenor.co": MEDIA
};

function findHeader(headers: Record<string, string[]>, name: string): string | undefined {
    return Object.keys(headers).find(key => key.toLowerCase() === name);
}

function parseCsp(value: string): Record<string, string[]> {
    const directives: Record<string, string[]> = {};
    for (const segment of value.split(";")) {
        const [name, ...values] = segment.trim().split(/\s+/g);
        if (name && !Object.prototype.hasOwnProperty.call(directives, name)) directives[name] = values;
    }
    return directives;
}

function serializeCsp(directives: Record<string, string[]>): string {
    return Object.entries(directives)
        .filter(([, values]) => values?.length)
        .map(([name, values]) => `${name} ${values.join(" ")}`)
        .join("; ");
}

export function registerCspOverrides(): void {
    session.defaultSession.webRequest.onHeadersReceived((details, callback) => {
        const { responseHeaders, resourceType } = details;

        if (responseHeaders && resourceType === "mainFrame") {
            const reportOnly = findHeader(responseHeaders, "content-security-policy-report-only");
            if (reportOnly) delete responseHeaders[reportOnly];

            const csp = findHeader(responseHeaders, "content-security-policy");
            if (csp) {
                const directives = parseCsp(responseHeaders[csp][0]);
                const add = (name: string, ...values: string[]) => {
                    directives[name] ??= [...(directives["default-src"] ?? [])];
                    directives[name].push(...values);
                    // style-src-elem/style-src-attr shadow style-src when present
                    if (name === "style-src") {
                        for (const variant of ["style-src-elem", "style-src-attr"]) {
                            directives[variant] ??= [...(directives["default-src"] ?? [])];
                            directives[variant].push(...values);
                        }
                    }
                };

                add("style-src", "'unsafe-inline'");
                add("script-src", "'unsafe-inline'", "'unsafe-eval'");
                for (const directive of CSP_DIRECTIVES) add(directive, "blob:", "data:", "vegord:", "vegord:");
                for (const [domain, directivesFor] of Object.entries(DEFAULT_ALLOWED)) {
                    for (const directive of directivesFor) add(directive, domain);
                }

                responseHeaders[csp] = [serializeCsp(directives)];
            }
        }

        callback({ cancel: false, responseHeaders });
    });
}
