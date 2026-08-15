/*
 * vegord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2023 Vendicated and vegord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import type { Rectangle } from "electron";

export interface Settings {
    discordBranch?: "stable" | "canary" | "ptb";
    transparencyOption?: "none" | "mica" | "tabbed" | "acrylic";
    webRTCIPHandlingPolicy?:
        | "default"
        | "default_public_interface_only"
        | "default_public_and_private_interfaces"
        | "disable_non_proxied_udp";
    tray?: boolean;
    minimizeToTray?: boolean;
    autoStartMinimized?: boolean;
    openLinksWithElectron?: boolean;
    staticTitle?: boolean;
    enableMenu?: boolean;
    disableSmoothScroll?: boolean;
    hardwareAcceleration?: boolean;
    hardwareVideoAcceleration?: boolean;
    arRPC?: boolean;
    appBadge?: boolean;
    enableTaskbarFlashing?: boolean;
    disableMinSize?: boolean;
    clickTrayToShowHide?: boolean;
    customTitleBar?: boolean;

    enableSplashScreen?: boolean;
    splashTheming?: boolean;
    splashColor?: string;
    splashBackground?: string;
    splashPixelated?: boolean;

    spellCheckLanguages?: string[];

    enableTelemetry?: boolean;
    shareDiscordUsername?: boolean;

    /** Upstream SOCKS5 relay ("user:pass@host:port") for Discord traffic, to bypass the GFW's Cloudflare-Spectrum relay that rejects Discord with Cloudflare error 1034. */
    relaySocks5?: string;

    /** Discord-only split tunnel via tun2proxy (default off). Requires CAP_NET_ADMIN on the gfw_proxy and tun2proxy binaries (applied by setcap at package install). When on, the proxy spawns tun2proxy, routes only Discord IPs into the TUN, and lets WebRTC UDP flow direct into the tunnel instead of via SOCKS. */
    discordTunTunnel?: boolean;

    /** Terminate Discord TLS locally with a self-signed cert and re-connect with the proxy's own rustls stack (default off). App runs with --ignore-certificate-errors to accept the local cert. This defeats DPI that fingerprint-matches Chromium/BoringSSL even through the proxy. */
    tlsMitm?: boolean;

    audio?: {
        workaround?: boolean;

        deviceSelect?: boolean;
        granularSelect?: boolean;

        ignoreVirtual?: boolean;
        ignoreDevices?: boolean;
        ignoreInputMedia?: boolean;

        mute?: boolean;
        onlySpeakers?: boolean;
        onlyDefaultSpeakers?: boolean;
    };
}

export interface State {
    maximized?: boolean;
    minimized?: boolean;
    windowBounds?: Rectangle;

    firstLaunch?: boolean;

    steamOSLayoutVersion?: number;
    linuxAutoStartEnabled?: boolean;

    vegordDir?: string;

    updater?: {
        ignoredVersion?: string;
        snoozeUntil?: number;
    };

    telemetry?: {
        clientId?: string;
    };

    settingsSync?: {
        pendingSecret?: string;
        users?: Record<
            string,
            {
                secret?: string;
                lastRestoreDay?: string;
            }
        >;
    };
}
