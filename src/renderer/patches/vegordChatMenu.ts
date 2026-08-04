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
 * button is injected straight into the chat bar DOM and the expression-picker
 * tabs get clones of Discord's own chat bar icons (the GIF/sticker/gift/apps
 * buttons are hidden once our ⋮ button is present).
 *
 * The renderer runs from the preload (webFrame.executeJavaScript) before
 * Discord's real document exists, so a MutationObserver attached once to
 * documentElement dies with the initial document. A poller therefore
 * re-attaches the observer to the live document on every tick and also scans
 * on a timer, so injection works regardless of when Discord renders its UI.
 */

type UploadState =
    | { status: "uploading"; name: string; sent: number; total: number }
    | { status: "done"; url: string; name: string }
    | { status: "error"; message: string };

const CHAT_BAR_SELECTOR = '[class*="channelTextArea"]';
const MORE_BUTTON_ID = "vegord-more-button";
const MENU_ID = "vegord-more-menu";
const PANEL_ID = "vegord-upload-panel";

let upload: UploadState | null = null;
let panelTimer: ReturnType<typeof setTimeout> | null = null;
let menuOpen = false;
let buttonAnchor: HTMLElement | null = null;

function moreIconSvg() {
    return `<svg width="20" height="20" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M6 10c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2zm12 0c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2zm-6 0c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2z"/></svg>`;
}

function insertIntoChat(text: string) {
    const box = document.querySelector<HTMLElement>(`${CHAT_BAR_SELECTOR} [role="textbox"]`);
    if (!box) return false;
    box.focus();
    if (box instanceof HTMLTextAreaElement) {
        const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set;
        setter?.call(box, box.value + text);
        box.dispatchEvent(new Event("input", { bubbles: true }));
        return true;
    }
    document.execCommand("insertText", false, text);
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
    panel.style.cssText =
        "position:fixed;bottom:90px;right:16px;z-index:99999;width:360px;max-width:calc(100vw - 32px);" +
        "padding:12px;background:var(--background-secondary);border-radius:8px;" +
        "border:1px solid var(--background-modifier-accent);box-shadow:0 8px 16px rgba(0,0,0,.24);" +
        'font-family:var(--font-primary),"Twemoji Mozilla";user-select:none;box-sizing:border-box;';

    if (upload.status === "uploading") {
        const pct = upload.total > 0 ? Math.min(100, Math.round((upload.sent / upload.total) * 100)) : 0;
        panel.innerHTML = `
            <div style="display:flex;gap:10px;align-items:center;">
                <span style="font-size:18px">\u23f3</span>
                <div style="flex:1;min-width:0;">
                    <div style="font-size:13px;font-weight:600;color:var(--text-normal);overflow:hidden;text-overflow:ellipsis;white-space:nowrap;">${escapeHtml(upload.name)}</div>
                    <div style="margin-top:4px;height:6px;background:var(--background-modifier-accent);border-radius:3px;overflow:hidden;">
                        <div style="height:100%;width:${pct}%;background:var(--brand-500);transition:width .2s ease;"></div>
                    </div>
                </div>
                <div style="font-size:12px;color:var(--text-muted);white-space:nowrap;">${pct}%</div>
            </div>`;
    } else if (upload.status === "done") {
        panel.innerHTML = `
            <div style="display:flex;gap:10px;align-items:center;">
                <span style="font-size:18px;color:var(--green-360)">\u2714</span>
                <div style="flex:1;min-width:0;">
                    <div style="font-size:13px;font-weight:600;color:var(--text-normal)">Uploaded</div>
                    <div title="Copy link" style="font-size:12px;color:var(--brand-360);overflow:hidden;text-overflow:ellipsis;white-space:nowrap;cursor:pointer;">${escapeHtml(upload.name)}</div>
                </div>
                <button data-close style="background:none;border:none;color:var(--interactive-normal);cursor:pointer;font-size:16px;padding:2px 6px;">\u2715</button>
            </div>`;
        const doneUrl = upload.url;
        panel.querySelector<HTMLElement>("[title='Copy link']")!.addEventListener("click", () => {
            navigator.clipboard?.writeText(doneUrl);
        });
    } else {
        panel.innerHTML = `
            <div style="display:flex;gap:10px;align-items:center;">
                <span style="font-size:18px;color:var(--red-360)">\u2716</span>
                <div style="flex:1;min-width:0;">
                    <div style="font-size:13px;font-weight:600;color:var(--text-normal)">Upload failed</div>
                    <div style="font-size:12px;color:var(--text-muted);overflow:hidden;text-overflow:ellipsis;white-space:nowrap;">${escapeHtml(upload.message)}</div>
                </div>
                <button data-retry style="background:var(--background-modifier-hover);border:none;border-radius:4px;color:var(--interactive-normal);cursor:pointer;font-size:13px;padding:4px 10px;">Retry</button>
                <button data-close style="background:none;border:none;color:var(--interactive-normal);cursor:pointer;font-size:16px;padding:2px 6px;">\u2715</button>
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
        setUpload({ status: "done", url: result.url, name: result.name });
        insertIntoChat(result.url);
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

    const rect = buttonAnchor.getBoundingClientRect();
    const menu = document.createElement("div");
    menu.id = MENU_ID;
    menu.style.cssText =
        "position:fixed;right:" +
        (window.innerWidth - rect.right) +
        "px;bottom:" +
        (window.innerHeight - rect.top) +
        "px;" +
        "z-index:99999;min-width:200px;padding:6px;background:var(--background-secondary);border-radius:8px;" +
        "border:1px solid var(--background-modifier-accent);box-shadow:0 8px 16px rgba(0,0,0,.24),0 4px 8px rgba(0,0,0,.16);" +
        'font-family:var(--font-primary),"Twemoji Mozilla";user-select:none;box-sizing:border-box;';

    for (const item of menuItems) {
        const row = document.createElement("div");
        row.style.cssText =
            "display:flex;align-items:center;gap:8px;padding:8px 12px;cursor:pointer;border-radius:6px;" +
            "font-size:14px;font-weight:500;color:var(--interactive-normal);white-space:nowrap;";
        row.addEventListener("mouseenter", () => {
            row.style.background = "var(--background-modifier-hover)";
            row.style.color = "var(--interactive-hover)";
        });
        row.addEventListener("mouseleave", () => {
            row.style.background = "transparent";
            row.style.color = "var(--interactive-normal)";
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
    button.addEventListener("click", e => {
        e.stopPropagation();
        buttonAnchor = button;
        toggleMenu();
    });

    const attachWrapper = chatBar.querySelector<HTMLElement>('[class*="attachWrapper"]');
    if (attachWrapper && attachWrapper.parentElement) {
        attachWrapper.after(button);
    } else {
        chatBar.appendChild(button);
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
