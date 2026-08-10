/*
 * vegord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2023 Vendicated and vegord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import { BuildContext, BuildOptions, context } from "esbuild";
import { execFileSync } from "child_process";
import { copyFile, mkdir, rm } from "fs/promises";
import { existsSync } from "fs";
import { join } from "path";

import vegordDep from "./vegordDep.mjs";
import { includeDirPlugin } from "./includeDirPlugin.mts";

const isDev = process.argv.includes("--dev");

const CommonOpts: BuildOptions = {
    minify: !isDev,
    bundle: true,
    sourcemap: "linked",
    logLevel: "info"
};

const NodeCommonOpts: BuildOptions = {
    ...CommonOpts,
    format: "cjs",
    platform: "node",
    external: ["electron"],
    target: ["esnext"],
    loader: {
        ".node": "file"
    },
    define: {
        IS_DEV: JSON.stringify(isDev)
    }
};

const contexts = [] as BuildContext[];
async function createContext(options: BuildOptions) {
    contexts.push(await context(options));
}

async function copyVenmic() {
    if (process.platform !== "linux") return;

    return Promise.all([
        copyFile(
            "./node_modules/@vencord/venmic/prebuilds/venmic-addon-linux-x64/node-napi-v7.node",
            "./static/dist/venmic-x64.node"
        ),
        copyFile(
            "./node_modules/@vencord/venmic/prebuilds/venmic-addon-linux-arm64/node-napi-v7.node",
            "./static/dist/venmic-arm64.node"
        )
    ]).catch(() => console.warn("Failed to copy venmic. Building without venmic support"));
}

async function buildRustProxy() {
    const cargo = process.platform === "win32" ? "cargo.exe" : "cargo";
    console.log("Building Rust GFW proxy (cargo build --release)...");
    try {
        execFileSync(cargo, ["build", "--release"], { cwd: "./gfw_proxy_rs", stdio: "inherit" });
        return true;
    } catch (err) {
        console.warn("Failed to build Rust GFW proxy, falling back to prebuilt binary:", (err as Error).message);
        return false;
    }
}

// The Rust proxy is the only backend: static/gfw_proxy is rebuilt from scratch
// every time and contains just the native binary (the Python fallback was
// removed, see gfwProxy.ts).
async function copyGfwProxyWrapper() {
    const dest = "./static/gfw_proxy";
    await rm(dest, { recursive: true, force: true });
    await mkdir(dest, { recursive: true });

    const ok = await buildRustProxy();
    const binName = process.platform === "win32" ? "gfw_proxy.exe" : "gfw_proxy";
    const rustBin = join("gfw_proxy_rs", "target", "release", binName);
    if (ok && existsSync(rustBin)) {
        await copyFile(rustBin, join(dest, binName));
        console.log(`Copied compiled Rust gfw_proxy binary (${binName}) into static/gfw_proxy/`);
        return;
    }
    throw new Error("Rust proxy build failed and no prebuilt binary exists; refusing to ship without a proxy");
}

async function copyLibvegord() {
    if (process.platform !== "linux") return;

    try {
        await copyFile(
            "./packages/libvegord/build/Release/vegord.node",
            `./static/dist/libvegord-${process.arch}.node`
        );
        console.log("Using local libvegord build");
    } catch {
        console.log(
            "Using prebuilt libvegord binaries. Run `pnpm buildLibvegord` and build again to build from source - see README.md for more details"
        );
        return Promise.all([
            copyFile("./packages/libvegord/prebuilds/vegord-x64.node", "./static/dist/libvegord-x64.node"),
            copyFile("./packages/libvegord/prebuilds/vegord-arm64.node", "./static/dist/libvegord-arm64.node")
        ]).catch(() => console.warn("Failed to copy libvegord. Building without libvegord support"));
    }
}

await Promise.all([
    copyVenmic(),
    copyLibvegord(),
    copyGfwProxyWrapper(),
    createContext({
        ...NodeCommonOpts,
        entryPoints: ["src/main/index.ts"],
        outfile: "dist/js/main.js",
        footer: { js: "//# sourceURL=vegordMain" }
    }),
    createContext({
        ...NodeCommonOpts,
        entryPoints: ["src/main/arrpc/worker.ts"],
        outfile: "dist/js/arRpcWorker.js",
        footer: { js: "//# sourceURL=vegordArRpcWorker" }
    }),
    createContext({
        ...NodeCommonOpts,
        entryPoints: ["src/preload/index.ts"],
        outfile: "dist/js/preload.js",
        footer: { js: "//# sourceURL=vegordPreload" }
    }),
    createContext({
        ...NodeCommonOpts,
        entryPoints: ["src/preload/splash.ts"],
        outfile: "dist/js/splashPreload.js",
        footer: { js: "//# sourceURL=vegordSplashPreload" }
    }),
    createContext({
        ...NodeCommonOpts,
        entryPoints: ["src/preload/debug.ts"],
        outfile: "dist/js/debugPreload.js",
        footer: { js: "//# sourceURL=vegordDebugPreload" }
    }),
    createContext({
        ...NodeCommonOpts,
        entryPoints: ["src/preload/updater.ts"],
        outfile: "dist/js/updaterPreload.js",
        footer: { js: "//# sourceURL=vegordUpdaterPreload" }
    }),
    createContext({
        ...CommonOpts,
        globalName: "vegordApp",
        entryPoints: ["src/renderer/index.ts"],
        outfile: "dist/js/renderer.js",
        format: "iife",
        inject: ["./scripts/build/injectReact.mjs"],
        jsxFactory: "vegordCreateElement",
        jsxFragment: "vegordFragment",
        external: ["@vencord/types/*"],
        plugins: [vegordDep, includeDirPlugin("patches", "src/renderer/patches")],
        footer: { js: "//# sourceURL=vegordRenderer" }
    })
]);

const watch = process.argv.includes("--watch");

if (watch) {
    await Promise.all(contexts.map(ctx => ctx.watch()));
} else {
    await Promise.all(
        contexts.map(async ctx => {
            await ctx.rebuild();
            await ctx.dispose();
        })
    );
}
