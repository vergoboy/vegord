# Vesktop GFW Proxy

Fork of [Vesktop](https://github.com/Vencord/Vesktop) with a built-in GFW-resistant SOCKS5 proxy for regions where Discord is throttled or blocked.

## Features

- **Everything from upstream Vesktop**: Vencord preinstalled, lightweight, Linux screenshare with sound & Wayland
- **Built-in GFW-resistant proxy**: SOCKS5 proxy (`127.0.0.1:4500`) with DNS-over-HTTPS (DoH), TCP fragmentation, and smart Discord IP routing
- **Auto-starts** with Vesktop on Linux (use `--no-proxy` to disable)
- **Voice-optimized**: Longer timeouts for voice/TURN connections, WebRTC forced through proxy
- **No conflict** with original Vesktop — installs as `vesktop-gfw`

## Installation

### Arch Linux

```sh
# From the project directory:
makepkg -si

# Or install pre-built package:
sudo pacman -U vesktop-gfw-proxy-*.pkg.tar.zst
```

### AppImage

```sh
# Build with electron-builder:
pnpm package --linux AppImage
# Or download from Releases
```

### Windows

```sh
pnpm package --win
# Output in dist/ as NSIS installer or ZIP
```

### Build from Source

```sh
git clone https://github.com/Vencord/Vesktop
cd Vesktop
pnpm install
pnpm build
# Run without packaging:
electron .
# Or package:
pnpm package
```

## Usage

```sh
# Run with proxy (default):
vesktop-gfw

# Run without proxy (uses original Vesktop behavior):
vesktop-gfw --no-proxy

# Use a custom proxy instead of the built-in one:
vesktop-gfw --proxy-server="http://127.0.0.1:8080"
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

The proxy (`pyprox_HTTPS_v3.0.py`) runs as a background Python process:

1. **SOCKS5 on `127.0.0.1:4500`** — Electron routes all Discord traffic through it
2. **DNS-over-HTTPS (DoH)** — Resolves domains via 37+ DoH servers (Cloudflare, Google, Quad9, etc.) to bypass DNS poisoning
3. **TCP fragmentation** — Splits initial TLS handshake data into fragments to evade DPI (Deep Packet Inspection)
4. **Smart Discord routing** — Pings discovered Discord IPs and routes to the fastest one
5. **Offline DNS cache** — Hardcoded IPs for Twitter, Instagram, WhatsApp, YouTube, Facebook, Google as fallback

### Configuration

Edit `gfw_resist_HTTPS_proxy/pyprox_HTTPS_v3.0.py`:

| Setting | Default | Purpose |
|---------|---------|---------|
| `listen_PORT` | 4500 | Local proxy port |
| `num_fragment` | 12 | TCP fragment count (lower = less latency) |
| `fragment_sleep` | 0.002s | Delay between fragments |
| `my_socket_timeout` | 60s | General connection timeout |
| `voice_socket_timeout` | 120s | Voice/TURN connection timeout |
| `doh_timeout` | 5s | DoH query timeout |
| `discord_ping_interval` | 10s | How often to re-ping Discord IPs |

## Voice Troubleshooting

If voice channels fail or have high ping:

1. **Check logs**: Run from terminal and look for `[VOICE]` tags
2. **WebRTC internals**: Open `chrome://webrtc-internals` in Vesktop (from Vencord dev tools)
3. **Try without proxy**: `vesktop-gfw --no-proxy` to isolate if proxy is the issue
4. **Adjust WebRTC policy**: Settings → WebRTC IP Handling Policy → `disable_non_proxied_udp` (default when proxy is active)
5. **Socket timeout**: If voice connects but drops after ~60s, increase `voice_socket_timeout` in the config
6. **Discord IP routing**: The proxy automatically finds the best Discord IP. Watch for `[DISCORD]` log entries showing best IP updates.

### Voice Logs

The proxy logs voice-specific events with these tags:

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
Vesktop (Electron)
  ├── main.ts
  │   ├── cli.ts           ← CLI flag parsing
  │   ├── gfwProxy.ts      ← Proxy lifecycle (start/stop/getAddress)
  │   ├── mainWindow.ts    ← BrowserWindow creation
  │   └── ...
  ├── pyprox_HTTPS_v3.0.py ← SOCKS5 proxy process
  │   ├── DNS_over_Fragment ← DoH query engine
  │   ├── ThreadedServer   ← SOCKS5/HTTP proxy server
  │   ├── Discord pinger   ← Best IP discovery
  │   └── Log writer       ← Traffic stats & health
  ├── static/gfw_proxy/    ← Proxy files (copied during build)
  └── PKGBUILD             ← Arch Linux package definition
```

## Building Packages

```sh
# Arch Linux
makepkg -si

# AppImage
pnpm package --linux AppImage

# Windows (requires wine + cross-compilation deps)
pnpm package --win

# All Linux targets
pnpm package --linux
```

Output appears in the `dist/` directory.

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

The proxy script is at `gfw_resist_HTTPS_proxy/pyprox_HTTPS_v3.0.py`. During build, it's copied to `static/gfw_proxy/`. For rapid testing:

```sh
# Run the proxy standalone:
python3 gfw_resist_HTTPS_proxy/pyprox_HTTPS_v3.0.py

# In another terminal, test with curl:
curl --proxy socks5://127.0.0.1:4500 https://discord.com
```

## License

GPL-3.0-or-later — same as upstream Vesktop.
