/*
 * Vegcord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2023 Vendicated and Vencord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import { BrowserWindow } from "electron";
import { existsSync, readFileSync } from "fs";
import { join } from "path";

import { Settings } from "./settings";
import { DATA_DIR } from "./constants";

// Themed loading overlay injected into the Discord window while it loads,
// mirroring the splash screen (blobs, spinning logo, glass panel).
//
// Discord's own loading screen (the "app-spinner" element) already renders
// inside #app-mount, so "app-mount has children" must not be treated as
// "Discord is done loading" — otherwise the overlay would never show and the
// user would see the stock spinner. The overlay is injected over the native
// loading screen and only removed once the spinner is gone AND the window has
// actually been visible for a short grace period.

const OVERLAY_CSS = readFileSync(join(__dirname, "..", "..", "static", "views", "loadingOverlay.css"), "utf8");
let OVERLAY_HTML = readFileSync(join(__dirname, "..", "..", "static", "views", "loadingOverlay.html"), "utf8");

// Discord's CSP doesn't allow loading the vegord:// scheme inside its page, so
// inline the splash image as a data: URL (which img-src permits).
const splashPath = join(__dirname, "..", "..", "static", "splash.webp");
if (existsSync(splashPath)) {
    OVERLAY_HTML = OVERLAY_HTML.replace(
        "vegord://assets/splash",
        `data:image/webp;base64,${readFileSync(splashPath).toString("base64")}`
    );
}

// How long the overlay must stay visible after the window appears before it
// is allowed to disappear (in ms). Guarantees the themed loading page is
// always noticeable, even when Discord mounted while the window was hidden.
const MIN_VISIBLE_MS = 1000;

const FACT_PLACEHOLDER = "{{FUN_FACT}}";

// Fun facts are served from a JSON "database" so the pool can grow without
// touching code. The app ships with static/funFacts.json; users can extend it
// by dropping more facts (a plain JSON array of strings) into
// <data dir>/funFacts.json.
const loadFacts = (file: string): string[] => {
    try {
        const parsed = JSON.parse(readFileSync(file, "utf8")) as unknown;
        if (Array.isArray(parsed)) {
            return parsed.filter((fact): fact is string => typeof fact === "string" && fact.trim().length > 0);
        }
    } catch {}
    return [];
};

const FUN_FACTS = [
    ...loadFacts(join(__dirname, "..", "..", "static", "funFacts.json")),
    ...loadFacts(join(DATA_DIR, "funFacts.json"))
];

let lastFact: string | undefined;

const pickFact = () => {
    if (FUN_FACTS.length === 0) return "Did you know? Loading screens are the internet's thinking face.";
    if (FUN_FACTS.length === 1) return FUN_FACTS[0];
    let fact = FUN_FACTS[Math.floor(Math.random() * FUN_FACTS.length)];
    if (fact === lastFact) fact = FUN_FACTS[(FUN_FACTS.indexOf(fact) + 1) % FUN_FACTS.length];
    lastFact = fact;
    return fact;
};

const INSTALL_SCRIPT = (html: string) => `(() => {
    if (document.getElementById("vegord-loading")) return "installed";
    const mount = document.getElementById("app-mount");
    // Discord's own loading screen (spinner) lives inside #app-mount, so only
    // consider it "ready" once that spinner is gone.
    const ready = mount && mount.children.length > 0 && !mount.querySelector("[data-testid='app-spinner']");
    if (ready) return "ready";
    const el = document.createElement("div");
    el.id = "vegord-loading";
    el.innerHTML = ${JSON.stringify(html)};
    const root = document.body || document.documentElement;
    if (root) root.appendChild(el);
    return "installed";
})()`;

const MARK_VISIBLE_SCRIPT = `window.__vegordLoadingVisibleSince = Date.now()`;

const POLL_SCRIPT = `(() => {
    const el = document.getElementById("vegord-loading");
    if (!el) return "gone";
    const mount = document.getElementById("app-mount");
    const since = window.__vegordLoadingVisibleSince;
    const spinner = mount && mount.querySelector("[data-testid='app-spinner']");
    if (mount && mount.children.length > 0 && !spinner && since && Date.now() - since > ${MIN_VISIBLE_MS}) {
        el.remove();
        return "gone";
    }
    return "waiting";
})()`;

export function themeDiscordLoadingScreen(win: BrowserWindow) {
    const { splashBackground, splashColor, splashTheming, splashPixelated } = Settings.store;
    if (splashTheming === false) return;

    let varCss = "";
    if (splashColor) {
        const semiTransparentSplashColor = splashColor.replace("rgb(", "rgba(").replace(")", ", 0.2)");
        varCss += `body { --vegord-fg: ${splashColor} !important; --vegord-fg-semi: ${semiTransparentSplashColor} !important; }`;
    }
    if (splashBackground) {
        varCss += `body { --vegord-bg: ${splashBackground} !important; }`;
    }
    if (splashPixelated) {
        varCss += `#vegord-loading img { image-rendering: pixelated; }`;
    }

    let poller: NodeJS.Timeout | undefined;

    const clearPoller = () => {
        if (poller) {
            clearInterval(poller);
            poller = undefined;
        }
    };

    const markVisible = () => {
        if (win.isDestroyed() || win.webContents.isDestroyed()) return;
        win.webContents.executeJavaScript(MARK_VISIBLE_SCRIPT).catch(() => {});
    };

    // The overlay may only disappear once the window has been visible
    win.on("show", markVisible);

    const inject = () => {
        if (win.isDestroyed() || win.webContents.isDestroyed()) return;
        clearPoller();

        win.webContents.insertCSS(OVERLAY_CSS).catch(() => {});
        if (varCss) win.webContents.insertCSS(varCss).catch(() => {});

        win.webContents
            .executeJavaScript(INSTALL_SCRIPT(OVERLAY_HTML.replace(FACT_PLACEHOLDER, pickFact())))
            .then(status => {
                if (win.isDestroyed()) return;
                if (status === "ready") return; // Discord already mounted, nothing to show
                if (status !== "installed") return;

                // Window may already be visible (splash screen disabled)
                if (win.isVisible()) markVisible();

                poller = setInterval(() => {
                    if (win.isDestroyed()) return clearPoller();
                    win.webContents
                        .executeJavaScript(POLL_SCRIPT)
                        .then(result => {
                            if (result === "gone") clearPoller();
                        })
                        .catch(() => {});
                }, 250);
            })
            .catch(() => {});
    };

    const removeOverlay = () => {
        if (win.isDestroyed() || win.webContents.isDestroyed()) return;
        clearPoller();
        win.webContents.executeJavaScript(`document.getElementById("vegord-loading")?.remove()`).catch(() => {});
    };

    // Re-inject on every navigation (retries, reloads, redirects reset the document)
    win.webContents.on("did-start-loading", inject);
    win.webContents.on("dom-ready", inject);

    // On a hard load failure the error page should be visible, so drop the overlay
    win.webContents.on("did-fail-load", (_event, errorCode, _errorDescription, _validatedURL, isMainFrame) => {
        if (isMainFrame && errorCode !== -3) removeOverlay();
    });

    inject();
}
