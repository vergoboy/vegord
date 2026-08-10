/*
 * vegord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2025 Vendicated and vegord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import { app } from "electron";
import { readFileSync } from "fs";
import { basename, join } from "path";
import { stripIndent } from "shared/utils/text";
import { parseArgs, ParseArgsOptionDescriptor } from "util";

type Option = ParseArgsOptionDescriptor & {
    description: string;
    hidden?: boolean;
    options?: string[];
    argumentName?: string;
};

const options = {
    "start-minimized": {
        default: false,
        type: "boolean",
        short: "m",
        description: "Start the application minimized to the system tray"
    },
    version: {
        type: "boolean",
        short: "v",
        description: "Print the application version and exit"
    },
    help: {
        type: "boolean",
        short: "h",
        description: "Print help information and exit"
    },
    "user-agent": {
        type: "string",
        argumentName: "ua",
        description: "Set a custom User-Agent. May trigger anti-spam or break voice chat"
    },
    "user-agent-os": {
        type: "string",
        description: "Set User-Agent to a specific operating system. May trigger anti-spam or break voice chat",
        options: ["windows", "linux", "darwin"]
    },
    repair: {
        type: "boolean",
        description: "Repair the application by re-downloading the latest vegord files"
    }
} satisfies Record<string, Option>;

// only for help display
const extraOptions = {
    "enable-features": {
        type: "string",
        description: "Enable specific Chromium features",
        argumentName: "feature1,feature2,…"
    },
    "disable-features": {
        type: "string",
        description: "Disable specific Chromium features",
        argumentName: "feature1,feature2,…"
    },
    "ozone-platform": {
        hidden: process.platform !== "linux",
        type: "string",
        description: "Whether to run vegord in Wayland or X11 (XWayland)",
        options: ["x11", "wayland"]
    }
} satisfies Record<string, Option>;

// The packaged app runs as `electron dist/js/main.js`, so Electron would treat
// it as the default "Electron" app and use ~/.config/Electron for userData.
// Pin the app name and data directory to "vegord" so settings, session and
// logs consistently land in ~/.config/vegord no matter how the app is started.
// Must run before constants.ts resolves DATA_DIR. cli.ts is imported first by
// main.ts/index.ts and by constants.ts itself, so this is always evaluated in
// time. An explicit --user-data-dir is left untouched.
app.setName("vegord");
if (!app.commandLine.hasSwitch("user-data-dir")) {
    try {
        app.setPath("userData", join(app.getPath("appData"), "vegord"));
    } catch {}
}
// Renaming the app above makes Electron's default-app fallback report version
// "0.0" instead of the Electron version, which is not valid semver and makes
// electron-updater throw at startup. Pin the real version from our package.json.
try {
    const { version } = JSON.parse(readFileSync(join(__dirname, "..", "..", "package.json"), "utf8"));
    if (typeof version === "string") {
        // app.setVersion exists at runtime but is missing from Electron's typings
        (app as unknown as { setVersion(v: string): void }).setVersion(version);
    }
} catch {}

const args = basename(process.argv[0]).toLowerCase().startsWith("electron")
    ? process.argv.slice(2)
    : process.argv.slice(1);

export const CommandLine = parseArgs({
    args,
    options,
    strict: false as true, // we manually check later, so cast to true to get better types
    allowPositionals: true
});

export function checkCommandLineForHelpOrVersion() {
    const { help, version } = CommandLine.values;

    if (version) {
        console.log(`vegord v${app.getVersion()}`);
        app.exit(0);
    }

    if (help) {
        const base = stripIndent`
            vegord v${app.getVersion()}

            Usage: ${basename(process.execPath)} [options] [url]

            Electron Options:
              See <https://www.electronjs.org/docs/latest/api/command-line-switches#electron-cli-flags>

            Chromium Options:
              See <https://peter.sh/experiments/chromium-command-line-switches> - only some of them work

            vegord Options:
        `;

        const optionLines = Object.entries(options)
            .sort(([a], [b]) => a.localeCompare(b))
            .concat(Object.entries(extraOptions))
            .filter(([, opt]) => !("hidden" in opt && opt.hidden))
            .map(([name, opt]) => {
                const flags = [
                    "short" in opt && `-${opt.short}`,
                    `--${name}`,
                    opt.type !== "boolean" &&
                        ("options" in opt ? `<${opt.options.join(" | ")}>` : `<${opt.argumentName ?? opt.type}>`)
                ]
                    .filter(Boolean)
                    .join(" ");

                return [flags, opt.description];
            });

        const padding = optionLines.reduce((max, [flags]) => Math.max(max, flags.length), 0) + 4;

        const optionsHelp = optionLines
            .map(([flags, description]) => `  ${flags.padEnd(padding, " ")}${description}`)
            .join("\n");

        console.log(base + "\n" + optionsHelp);
        app.exit(0);
    }

    for (const [name, def] of Object.entries(options)) {
        const value = CommandLine.values[name];
        if (value == null) continue;

        if (typeof value !== def.type) {
            console.error(`Invalid options. Expected ${def.type === "boolean" ? "no" : "an"} argument for --${name}`);
            app.exit(1);
        }

        if ("options" in def && !def.options?.includes(value as string)) {
            console.error(`Invalid value for --${name}: ${value}\nExpected one of: ${def.options.join(", ")}`);
            app.exit(1);
        }
    }
}

checkCommandLineForHelpOrVersion();
