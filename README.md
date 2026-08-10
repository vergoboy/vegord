# Vegord

A custom Discord desktop app with a **built-in GFW-resistant proxy** for regions where Discord is throttled or blocked.

Fork of [Vencord](https://github.com/Vendicated/Vencord) with Vencord preinstalled plus a high-performance Rust SOCKS5/HTTP proxy that tunnels Discord traffic past Deep Packet Inspection and DNS poisoning.

## Features

- **Everything from upstream vegord**: vegord preinstalled, lightweight, Linux screenshare with sound & Wayland
- **Built-in GFW-resistant proxy**: high-performance Rust proxy (`gfw_proxy_rs`) exposing SOCKS5 on `127.0.0.1:4500`
- **DNS-over-HTTPS (DoH)**: resolves via 37+ DoH servers (Cloudflare, Google, Quad9, ...) to bypass DNS poisoning
- **TCP fragmentation**: splits the TLS ClientHello into fragments to evade DPI
- **Smart Discord IP routing**: pings discovered Discord IPs and routes to the fastest one, with an offline DNS cache as fallback
- **Voice-optimized**: longer timeouts for voice/TURN connections, WebRTC forced through the proxy
- **Auto-starts** on launch (use `--no-proxy` to disable)
- **No conflict** with upstream vegord — separate app/executable names

## Installation

### Windows

Grab the latest NSIS installer (`vegord Setup <version>.exe`) from the [Releases](https://github.com/vergoboy/vegord/releases) page, or the portable ZIP (`vegord-<version>-win.zip` for x64, `-arm64-win.zip` for ARM64).

### Arch Linux

```sh
sudo pacman -U vegord-gfw-proxy-*.pkg.tar.zst
# Run with: vegord (proxy on by default), vegord-gfw, or vegord-gfw-proxy
```

### AppImage

Download from Releases, or build it yourself:

```sh
pnpm package --linux AppImage
```

### Build from Source

```sh
git clone https://github.com/vergoboy/vegord
cd vegord
pnpm install
pnpm build
# Run without packaging:
electron .
# Or package for the current platform:
pnpm package
```

#### Cross-compiling for Windows on Linux

The NSIS/ZIP Windows targets can be built on Linux. You need:

```sh
sudo pacman -S mingw-w64-gcc   # or your distro's mingw-w64-gcc
rustup target add x86_64-pc-windows-gnu
# wine is required for electron-builder to stamp the executable (icon/version)
```

Then:

```sh
pnpm build
# Build the Windows gfw_proxy.exe into static/gfw_proxy/:
cargo build --release --target x86_64-pc-windows-gnu --manifest-path gfw_proxy_rs/Cargo.toml
cp gfw_proxy_rs/target/x86_64-pc-windows-gnu/release/gfw_proxy.exe static/gfw_proxy/
# Package:
npx electron-builder --win
```

Outputs land in `dist/` (`vegord Setup <version>.exe`, `vegord-<version>-win.zip`, ...).

## Usage

```sh
# Run with the built-in proxy (default):
vegord

# Run without the proxy (plain vegord behavior):
vegord --no-proxy

# Use a custom proxy instead of the built-in one:
vegord --proxy-server="http://127.0.0.1:8080"
```

### Command-Line Flags

| Flag | Description |
|------|-------------|
| `--no-proxy` | Disable the built-in GFW proxy |
| `--proxy-server <addr>` | Use a custom proxy (overrides built-in) |
| `--start-minimized`, `-m` | Start minimized to tray |
| `--help`, `-h` | Show help |
| `--version`, `-v` | Show version |

## How the Proxy Works

On startup the app spawns the Rust binary `gfw_proxy` (`gfw_proxy.exe` on Windows) from `static/gfw_proxy/`.

1. **SOCKS5 on `127.0.0.1:4500`** — Electron routes all Discord traffic through it
2. **DNS-over-HTTPS (DoH)** — bypasses DNS poisoning via racing DoH servers
3. **TCP fragmentation** — splits the initial TLS handshake into fragments to evade DPI
4. **Smart Discord routing** — routes to the fastest discovered Discord IP
5. **Offline DNS cache** — hardcoded IPs for Twitter, Instagram, WhatsApp, YouTube, Facebook, Google as fallback

### Configuration

The Rust proxy is configured via CLI flags (`--port`, `--data-dir`) in `src/main/gfwProxy.ts` and environment variables (`VEGORD_PROXY_*`) documented in the proxy source:

| Setting | Default | Purpose |
|---------|---------|---------|
| `--port` | 4500 | Local proxy port |
| `--num-fragment` | 6 | TCP fragment count (lower = less latency) |
| `--fragment-sleep` | 1ms | Delay between fragments |
| `--control-token` | (none) | Auth token for the localhost control API |

## Voice Troubleshooting

If voice channels fail or have high ping:

1. **Check logs**: Run from terminal and look for `[GFW Proxy]` and `[VOICE]` tags
2. **WebRTC internals**: Open `chrome://webrtc-internals` in the app (from vegord dev tools)
3. **Try without proxy**: `vegord --no-proxy` to isolate if the proxy is the issue
4. **Adjust WebRTC policy**: Settings → WebRTC IP Handling Policy → `disable_non_proxied_udp` (default when proxy is active)
5. **Socket timeout**: If voice connects but drops after ~60s, increase `voice_socket_timeout`
6. **Discord IP routing**: The proxy automatically finds the best Discord IP. Watch for `[DISCORD]` log entries showing best IP updates

### Voice Log Tags

- `[VOICE] Connecting...` — Voice/TURN connection attempt
- `[VOICE] Connected in Xms` — Successful voice connection with latency
- `[VOICE TIMEOUT]` — Voice connection timed out (try increasing `voice_socket_timeout`)
- `[VOICE FILTERED]` — Connection blocked (try different IP)
- `[DISCORD] best IP updated` — Proxy found a faster Discord server

### Recommended Settings for Low Ping

| Setting | Gaming | Voice Call | Balanced |
|---------|--------|------------|----------|
| `num_fragment` | 3 | 5 | 12 |
| `fragment_sleep` | 0.5ms | 1ms | 2ms |
| `voice_socket_timeout` | 120s | 120s | 120s |
| `discord_ping_interval` | 5s | 10s | 10s |

## Architecture

```
Vegord (Electron)
  ├── src/main
  │   ├── gfwProxy.ts      ← Proxy lifecycle (start/stop, spawns Rust binary)
  │   ├── cli.ts           ← CLI flag parsing
  │   ├── mainWindow.ts    ← BrowserWindow creation
  │   └── ...
  ├── gfw_proxy_rs/        ← High-performance Rust proxy (SOCKS5/HTTP + DoH + fragment + voice UDP)
  ├── static/gfw_proxy/    ← Native proxy binary (copied during build, gitignored)
  ├── packages/libvegord/  ← Native addon (screenshare/venmic)
  └── PKGBUILD             ← Arch Linux package definition
```

## Development

```sh
pnpm build             # Build TypeScript & copy proxy files
pnpm start             # Build & run
pnpm build:dev         # Dev build (faster, no minify)
pnpm start:dev         # Dev build & run
pnpm start:watch       # Watch mode
pnpm lint              # ESLint
pnpm testTypes         # TypeScript type check
```

### Proxy Development

```sh
# Run the Rust proxy standalone:
cd gfw_proxy_rs && cargo run --release

# In another terminal, test with curl:
curl --proxy socks5://127.0.0.1:4500 https://discord.com
```

## License

GPL-3.0-or-later — same as upstream vegord.
