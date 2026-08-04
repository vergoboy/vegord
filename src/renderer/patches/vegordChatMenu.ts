/*
 * Vegcord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2026 Vendicated and Vegcord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

/*
 * Vegcord: "More" chat bar button + "Upload Vegord" + emoji-picker tab icons.
 *
 * Pure DOM implementation. It deliberately avoids Vencord's ChatButtons API: the
 * _injectButtons patch no longer matches modern Discord's chat bar, so buttons
 * registered via addChatBarButton are silently never rendered. Instead the ⋮
 * button is injected into the chat bar's right-side button cluster (next to the
 * emoji button, so its menu can open upward inside the chat card) and the
 * expression-picker tabs get clones of Discord's own chat bar icons.
 *
 * The renderer runs from the preload (webFrame.executeJavaScript) before
 * Discord's real document exists, so a MutationObserver attached once to
 * documentElement dies with the initial document. A poller therefore
 * re-attaches the observer to the live document on every tick and also scans
 * on a timer, so injection works regardless of when Discord renders its UI.
 *
 * The menu/upload panel are appended to <body>, outside #app-mount, where
 * Discord's theme CSS variables do not cascade; they use an explicit
 * semi-transparent surface with backdrop blur and hard-coded color fallbacks.
 */

type UploadState =
    | { status: "uploading"; name: string; sent: number; total: number }
    | { status: "done"; url: string; name: string }
    | { status: "error"; message: string };

const CHAT_BAR_SELECTOR = '[class*="channelTextArea"]';
const MORE_BUTTON_ID = "vegord-more-button";
const MENU_ID = "vegord-more-menu";
const PANEL_ID = "vegord-upload-panel";
const MENU_WIDTH = 220;

let upload: UploadState | null = null;
let panelTimer: ReturnType<typeof setTimeout> | null = null;
let menuOpen = false;
let buttonAnchor: HTMLElement | null = null;

/* Shared look for the fixed-position menu/panel: a blurred, semi-transparent
   surface so nothing behind it bleeds through and confuses the eye. */
const SURFACE_CSS =
    "position:fixed;z-index:99999;" +
    "background:rgba(30,31,34,.82);backdrop-filter:blur(10px);-webkit-backdrop-filter:blur(10px);" +
    "border:1px solid rgba(255,255,255,.08);box-shadow:0 8px 16px rgba(0,0,0,.35),0 4px 8px rgba(0,0,0,.2);" +
    "border-radius:8px;color:#dbdee1;user-select:none;box-sizing:border-box;" +
    'font-family:"Twemoji Mozilla",Whitney,"Helvetica Neue",Helvetica,Arial,sans-serif;';

function chatBarRect(): DOMRect | null {
    return document.querySelector<HTMLElement>(CHAT_BAR_SELECTOR)?.getBoundingClientRect() ?? null;
}

/* Anchor above the chat bar, inset a bit from its right edge so the panel stays
   inside the chat card instead of sliding over the member list. */
function panelPosition(): string {
    const r = chatBarRect();
    const right = r ? Math.max(8, window.innerWidth - r.right + 8) : 16;
    const bottom = r ? Math.max(8, window.innerHeight - r.top + 8) : 90;
    return `right:${right}px;bottom:${bottom}px;`;
}

/* Anchor above the ⋮ button, right-aligned to it so the menu opens inside the
   chat card. If the menu would overflow the window's left edge (e.g. when the
   button sits mid-bar), flip it to open rightward from the button instead. */
function menuPosition(): string {
    const r = buttonAnchor?.getBoundingClientRect();
    if (!r) return "right:16px;bottom:90px;";
    const bottom = Math.max(8, window.innerHeight - r.top);
    const fitsRight = r.right - MENU_WIDTH >= 8;
    const horiz = fitsRight ? `right:${Math.max(8, window.innerWidth - r.right)}px` : `left:${Math.max(8, r.left)}px`;
    return `${horiz};bottom:${bottom}px;`;
}

function moreIconSvg() {
    return `<svg width="20" height="20" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M6 10c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2zm12 0c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2zm-6 0c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2z"/></svg>`;
}

/* Feed a real File into Discord's composer as a native attachment preview,
   exactly like dropping a file onto the chat bar. */
function dropFileIntoComposer(file: File): boolean {
    const editor = document.querySelector<HTMLElement>(`${CHAT_BAR_SELECTOR} [role="textbox"]`);
    if (!editor) return false;
    const dt = new DataTransfer();
    dt.items.add(file);
    editor.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: dt }));
    return true;
}

/* ---------------------------- upload panel ---------------------------- */

VegcordNative.upload.onProgress(p => {
    if (upload?.status === "uploading") {
        upload = { ...upload, sent: p.sent, total: p.total || upload.total };
        renderUploadPanel();
    }
});

function renderUploadPanel() {
    document.getElementById(PANEL_ID)?.remove();
    if (!upload) return;

    const panel = document.createElement("div");
    panel.id = PANEL_ID;
    panel.style.cssText = SURFACE_CSS + panelPosition() + "width:360px;max-width:calc(100vw - 24px);padding:12px;";

    if (upload.status === "uploading") {
        const pct = upload.total > 0 ? Math.min(100, Math.round((upload.sent / upload.total) * 100)) : 0;
        panel.innerHTML = `
            <div style="display:flex;gap:10px;align-items:center;">
                <span style="font-size:18px">\u23f3</span>
                <div style="flex:1;min-width:0;">
                    <div style="font-size:13px;font-weight:600;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;">${escapeHtml(upload.name)}</div>
                    <div style="margin-top:4px;height:6px;background:rgba(255,255,255,.08);border-radius:3px;overflow:hidden;">
                        <div style="height:100%;width:${pct}%;background:#5865F2;transition:width .2s ease;"></div>
                    </div>
                </div>
                <div style="font-size:12px;color:#949ba4;white-space:nowrap;">${pct}%</div>
            </div>`;
    } else if (upload.status === "done") {
        panel.innerHTML = `
            <div style="display:flex;gap:10px;align-items:center;">
                <span style="font-size:18px;color:#23a559">\u2714</span>
                <div style="flex:1;min-width:0;">
                    <div style="font-size:13px;font-weight:600;">Uploaded</div>
                    <div style="font-size:12px;color:#949ba4;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;">Attachment added to the message box</div>
                </div>
                <button data-close style="background:none;border:none;color:#b5bac1;cursor:pointer;font-size:16px;padding:2px 6px;">\u2715</button>
            </div>`;
    } else {
        panel.innerHTML = `
            <div style="display:flex;gap:10px;align-items:center;">
                <span style="font-size:18px;color:#f23f43">\u2716</span>
                <div style="flex:1;min-width:0;">
                    <div style="font-size:13px;font-weight:600;">Upload failed</div>
                    <div style="font-size:12px;color:#949ba4;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;">${escapeHtml(upload.message)}</div>
                </div>
                <button data-retry style="background:rgba(255,255,255,.06);border:none;border-radius:4px;color:#dbdee1;cursor:pointer;font-size:13px;padding:4px 10px;">Retry</button>
                <button data-close style="background:none;border:none;color:#b5bac1;cursor:pointer;font-size:16px;padding:2px 6px;">\u2715</button>
            </div>`;
        panel.querySelector<HTMLElement>("[data-retry]")!.addEventListener("click", startUpload);
    }
    panel.querySelector<HTMLElement>("[data-close]")?.addEventListener("click", () => setUpload(null));
    document.body.appendChild(panel);
}

function setUpload(next: UploadState | null) {
    upload = next;
    if (panelTimer) {
        clearTimeout(panelTimer);
        panelTimer = null;
    }
    if (next?.status === "done") panelTimer = setTimeout(() => setUpload(null), 10_000);
    renderUploadPanel();
}

async function startUpload() {
    closeMenu();
    setUpload({ status: "uploading", name: "Preparing\u2026", sent: 0, total: 0 });
    try {
        const result = await VegcordNative.upload.pick();
        if (!result || "canceled" in result) {
            setUpload(null);
            return;
        }
        if ("error" in result) {
            setUpload({ status: "error", message: result.error });
            return;
        }
        setUpload({ status: "uploading", name: result.name, sent: 0, total: 0 });
        const blob = await fetch(result.url).then(r => {
            if (!r.ok) throw new Error(`HTTP ${r.status}`);
            return r.blob();
        });
        const file = new File([blob], result.name, { type: blob.type || "application/octet-stream" });
        if (!dropFileIntoComposer(file)) throw new Error("Could not find the message composer");
        setUpload({ status: "done", url: result.url, name: result.name });
    } catch (e) {
        setUpload({ status: "error", message: String(e) });
    }
}

/* ------------------------------- menu -------------------------------- */

function escapeHtml(s: string) {
    return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

function openNativeMoreMenuAndClick(label: string) {
    const plus = document.querySelector<HTMLElement>('[aria-label="More message options"]');
    if (!plus) return;
    plus.click();
    setTimeout(() => {
        const items = Array.from(document.querySelectorAll<HTMLElement>('[role="menuitem"], [class*="menuItem"]'));
        const item = items.find(el => (el.textContent || "").trim().startsWith(label));
        item?.click();
    }, 60);
}

const menuItems = [
    {
        icon: "\ud83d\udcce",
        label: "Upload file",
        action: () => document.querySelector<HTMLElement>(`${CHAT_BAR_SELECTOR} input[type="file"]`)?.click()
    },
    { icon: "\u2601\ufe0f", label: "Upload Vegord", action: startUpload },
    { icon: "\ud83d\udcca", label: "Create Poll", action: () => openNativeMoreMenuAndClick("Create Poll") },
    {
        icon: "\ud83c\udf81",
        label: "Send a Gift",
        action: () => document.querySelector<HTMLElement>('[aria-label="Give a Gift"]')?.click()
    },
    {
        icon: "\ud83e\udde9",
        label: "Apps",
        action: () => document.querySelector<HTMLElement>('[aria-label="Apps"]')?.click()
    }
];

function closeMenu() {
    menuOpen = false;
    document.getElementById(MENU_ID)?.remove();
    document.removeEventListener("click", onDocClick, true);
}

function onDocClick(e: MouseEvent) {
    const target = e.target as HTMLElement;
    if (target.id === MORE_BUTTON_ID || target.closest(`#${MORE_BUTTON_ID}`)) return;
    if (target.closest(`#${MENU_ID}`)) return;
    closeMenu();
}

function renderMenu() {
    document.getElementById(MENU_ID)?.remove();
    if (!menuOpen || !buttonAnchor) return;

    const menu = document.createElement("div");
    menu.id = MENU_ID;
    menu.style.cssText =
        SURFACE_CSS +
        menuPosition() +
        `min-width:${MENU_WIDTH}px;max-width:min(320px,calc(100vw - 24px));max-height:calc(100vh - 24px);overflow-y:auto;padding:6px;`;

    for (const item of menuItems) {
        const row = document.createElement("div");
        row.style.cssText =
            "display:flex;align-items:center;gap:8px;padding:8px 12px;cursor:pointer;border-radius:6px;" +
            "font-size:14px;font-weight:500;color:#b5bac1;white-space:nowrap;";
        row.addEventListener("mouseenter", () => {
            row.style.background = "rgba(255,255,255,.06)";
            row.style.color = "#dbdee1";
        });
        row.addEventListener("mouseleave", () => {
            row.style.background = "transparent";
            row.style.color = "#b5bac1";
        });
        row.addEventListener("click", () => item.action());
        row.innerHTML = `<span style="width:18px;text-align:center;font-size:16px;">${item.icon}</span><span>${item.label}</span>`;
        menu.appendChild(row);
    }
    document.body.appendChild(menu);
    document.addEventListener("click", onDocClick, true);
}

function toggleMenu() {
    menuOpen = !menuOpen;
    renderMenu();
}

/* ------------------------- chat bar button ------------------------- */

function injectButton(chatBar: HTMLElement) {
    if (chatBar.querySelector(`#${MORE_BUTTON_ID}`)) return;

    const button = document.createElement("div");
    button.id = MORE_BUTTON_ID;
    button.setAttribute("role", "button");
    button.setAttribute("aria-label", "Vegord menu");
    button.title = "More";
    button.innerHTML = moreIconSvg();
    button.style.cssText =
        "display:flex;align-items:center;justify-content:center;" +
        "width:36px;height:36px;flex:0 0 auto;border-radius:50%;cursor:pointer;" +
        "color:var(--interactive-normal,#b5bac1);background:transparent;" +
        "transition:background .15s ease,color .15s ease;";
    button.addEventListener("mouseenter", () => {
        button.style.background = "var(--background-modifier-hover,rgba(255,255,255,.06))";
        button.style.color = "var(--interactive-hover,#dbdee1)";
    });
    button.addEventListener("mouseleave", () => {
        button.style.background = "transparent";
        button.style.color = "var(--interactive-normal,#b5bac1)";
    });
    button.addEventListener("click", e => {
        e.stopPropagation();
        buttonAnchor = button;
        toggleMenu();
    });

    const emojiBtn = chatBar.querySelector<HTMLElement>('[aria-label="Open emoji picker"]');
    if (emojiBtn?.parentElement) {
        emojiBtn.parentElement.insertBefore(button, emojiBtn);
    } else {
        const attachWrapper = chatBar.querySelector<HTMLElement>('[class*="attachWrapper"]');
        if (attachWrapper?.parentElement) {
            attachWrapper.after(button);
        } else {
            chatBar.appendChild(button);
        }
    }
    document.body.classList.add("vegord-has-more-button");
}

function scanChatBar() {
    if (document.getElementById(MORE_BUTTON_ID)) return;
    const chatBar = document.querySelector<HTMLElement>(CHAT_BAR_SELECTOR);
    if (chatBar) injectButton(chatBar);
}

/* ------------------- emoji picker tab icons ------------------- */

const TAB_CHAT_BAR_LABELS: Record<string, string> = {
    emoji: "Open emoji picker",
    gif: "Open GIF picker",
    sticker: "Open sticker picker"
};

function tabSvgFromChatBar(kind: string): SVGSVGElement | null {
    const btn = document.querySelector<HTMLElement>(`[aria-label="${TAB_CHAT_BAR_LABELS[kind]}"]`);
    return btn?.querySelector<SVGSVGElement>("svg") ?? null;
}

function decorateEmojiPickerTabs() {
    const picker = document.querySelector<HTMLElement>('[class*="emojiPicker"]');
    if (!picker) return;
    for (const kind of ["emoji", "gif", "sticker"]) {
        const tab = picker.querySelector<HTMLElement>(`#${kind}-picker-tab`);
        if (!tab || tab.dataset.vegordTabIcon) continue;
        const icon = tabSvgFromChatBar(kind);
        if (!icon) continue;
        tab.dataset.vegordTabIcon = "1";
        tab.setAttribute("aria-label", (tab.textContent || kind).trim());
        tab.textContent = "";
        icon.style.width = "20px";
        icon.style.height = "20px";
        icon.style.display = "block";
        tab.appendChild(icon);
    }
}

/* --------------------- self-healing poller --------------------- */

let observedRoot: Node | null = null;
let pendingTick = false;

function tick() {
    pendingTick = false;
    scanChatBar();
    if (document.querySelector('[class*="emojiPicker"]')) decorateEmojiPickerTabs();
}

function scheduleTick() {
    if (pendingTick) return;
    pendingTick = true;
    setTimeout(tick, 120);
}

(function initPoller() {
    const observer = new MutationObserver(scheduleTick);
    const ensureObserving = () => {
        const root = document.documentElement;
        if (root && root !== observedRoot) {
            observedRoot = root;
            observer.observe(root, { childList: true, subtree: true });
        }
    };
    setInterval(() => {
        ensureObserving();
        tick();
    }, 800);
    ensureObserving();
    scheduleTick();
})();
