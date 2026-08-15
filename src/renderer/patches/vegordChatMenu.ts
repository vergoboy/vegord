/*
 * vegord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2026 Vendicated and vegord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

/*
 * vegord: "More" chat bar button + "Upload Vegord" + emoji-picker tab icons.
 *
 * Pure DOM implementation. It deliberately avoids vegord's ChatButtons API: the
 * _injectButtons patch no longer matches modern Discord's chat bar, so buttons
 * registered via addChatBarButton are silently never rendered. Instead the ⋮
 * button is injected inside the chat bar's attach wrapper (where the hidden
 * "+" button lived), so it matches the rest of the bar's buttons, and the
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
 * They are anchored to the chat card so they always stay inside it instead of
 * sliding over the channel list. Uploaded files come back from the main
 * process as a URL, which is inserted into the composer as a link (the file is
 * already hosted on vergoboy.ir, so it is sent as a link, not re-uploaded to
 * Discord's CDN).
 *
 * Manual QA (regression): pick a file via the widget and confirm (a) the URL
 * text appears in the composer (visible before hitting Send) and (b) sending
 * the message actually delivers the link to the channel.
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

/* Anchor above the chat bar, inset from its right edge so the panel stays
   inside the chat card instead of sliding over the member list. */
function panelPosition(): string {
    const r = chatBarRect();
    const right = r ? Math.max(8, window.innerWidth - r.right + 8) : 16;
    const bottom = r ? Math.max(8, window.innerHeight - r.top + 8) : 90;
    return `right:${right}px;bottom:${bottom}px;`;
}

/* Anchor the menu above the ⋮ button but keep it inside the chat card: if the
   button is in the left half of the bar the menu opens rightward (never over
   the channel/contact list), otherwise it opens upward right-aligned to the
   button. */
function menuPosition(): string {
    const b = buttonAnchor?.getBoundingClientRect();
    if (!b) return "right:16px;bottom:90px;";
    const bottom = Math.max(8, window.innerHeight - b.top);
    const bar = chatBarRect();
    if (bar) {
        const buttonCenter = b.left + b.width / 2;
        const barCenter = bar.left + bar.width / 2;
        if (buttonCenter < barCenter) {
            const left = Math.max(bar.left + 8, b.left);
            return `left:${left}px;bottom:${bottom}px;`;
        }
    }
    return `right:${Math.max(8, window.innerWidth - b.right)}px;bottom:${bottom}px;`;
}
function moreIconSvg() {
    return `<svg width="20" height="20" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M6 10c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2zm12 0c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2zm-6 0c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2z"/></svg>`;
}

/* Insert a URL into Discord's composer as plain text so sending the message
   includes it as a link — the file is already hosted on vergoboy.ir, so there
   is no need to re-upload it to Discord's CDN. The composer is a
   contenteditable editor: dispatch a beforeinput insertText (its inputType is
   what the editor listens for), falling back to the legacy execCommand path
   that fires the real input events, then verify the text actually landed. */
function insertLinkIntoComposer(url: string): Promise<boolean> {
    return new Promise(resolve => {
        const editor = document.querySelector<HTMLElement>(`${CHAT_BAR_SELECTOR} [role="textbox"]`);
        if (!editor) return resolve(false);

        editor.focus();
        const sel = window.getSelection();
        if (sel) {
            const range = document.createRange();
            range.selectNodeContents(editor);
            range.collapse(false);
            sel.removeAllRanges();
            sel.addRange(range);
        }

        const before = editor.textContent?.trim() ?? "";
        editor.dispatchEvent(
            new InputEvent("beforeinput", {
                inputType: "insertText",
                data: url,
                bubbles: true,
                cancelable: true,
                composed: true
            })
        );

        if ((editor.textContent?.trim() ?? "") === before) {
            document.execCommand("insertText", false, url);
        }

        // Discord's editor commits asynchronously, so only report success once
        // the composer actually contains the URL.
        setTimeout(() => {
            const after = editor.textContent?.trim() ?? "";
            resolve(after !== before && after.includes(url));
        }, 150);
    });
}

/* ---------------------------- upload panel ---------------------------- */

vegordNative.upload.onProgress(p => {
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
                    <div style="font-size:12px;color:#949ba4;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;">Link added to the message box</div>
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
        const result = await vegordNative.upload.pick();
        if (!result || "canceled" in result) {
            setUpload(null);
            return;
        }
        if ("error" in result) {
            setUpload({ status: "error", message: result.error });
            return;
        }
        setUpload({ status: "uploading", name: result.name, sent: 0, total: 0 });
        const inserted = await insertLinkIntoComposer(result.url);
        if (!inserted) throw new Error("Could not insert the link into the message box");
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
    document.body.classList.add("vegord-has-more-button");

    /* Sit inside the "+" (More message options) container, right where the
       hidden plus used to be. Falls back to before the emoji picker, then to
       the end of the bar. */
    const attachWrapper = chatBar.querySelector<HTMLElement>('[class*="attachWrapper"]');
    if (attachWrapper) {
        attachWrapper.prepend(button);
        return;
    }
    const emojiBtn = chatBar.querySelector<HTMLElement>('[aria-label="Open emoji picker"]');
    if (emojiBtn?.parentElement) {
        emojiBtn.parentElement.insertBefore(button, emojiBtn);
    } else {
        chatBar.appendChild(button);
    }
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
