#!/bin/bash
set -e
# Build an Arch Linux package from the current build directory
# Uses the same approach as makepkg for compatibility

pkgname=vegord-gfw-proxy
pkgver=1.7.3
pkgrel=1
pkgdir="/tmp/${pkgname}-pkg"
srcdir="$(cd "$(dirname "$0")" && pwd)"

rm -rf "$pkgdir"
mkdir -p "$pkgdir/opt/vegord/dist"
mkdir -p "$pkgdir/usr/share/icons/hicolor/scalable/apps"
mkdir -p "$pkgdir/usr/share/icons/hicolor/256x256/apps"
mkdir -p "$pkgdir/usr/share/applications"
mkdir -p "$pkgdir/usr/bin"
mkdir -p "$pkgdir/opt/vegord/static/gfw_proxy/logs"

# Ship the native Linux proxy binary. The Rust proxy is the only backend now,
# so build it with cargo; there is no prebuilt binary to fall back to.
GFW_BIN=""
if command -v cargo >/dev/null 2>&1; then
    if (cd "$srcdir/gfw_proxy_rs" && cargo build --release) >/dev/null 2>&1; then
        GFW_BIN="$srcdir/gfw_proxy_rs/target/release/gfw_proxy"
    fi
fi
if [ -n "$GFW_BIN" ] && [ -x "$GFW_BIN" ]; then
    cp "$GFW_BIN" "$srcdir/static/gfw_proxy/gfw_proxy"
    chmod +x "$srcdir/static/gfw_proxy/gfw_proxy"
    echo "Packaged native Linux proxy: $GFW_BIN"
else
    echo "ERROR: no Linux gfw_proxy binary found (cargo build failed). The app cannot proxy without it." >&2
    exit 1
fi

# Ship the tun2proxy relay for the Discord split tunnel (opt-in via the
# discordTunTunnel setting; requires CAP_NET_ADMIN granted by setcap at install).
TUN2PROXY_BIN=""
if [ -d "$srcdir/tun2proxy" ] && command -v cargo >/dev/null 2>&1; then
    if (cd "$srcdir/tun2proxy" && cargo build --release --bin tun2proxy-bin) >/dev/null 2>&1; then
        TUN2PROXY_BIN="$srcdir/tun2proxy/target/release/tun2proxy-bin"
    fi
fi
if [ -n "$TUN2PROXY_BIN" ] && [ -x "$TUN2PROXY_BIN" ]; then
    cp "$TUN2PROXY_BIN" "$srcdir/static/gfw_proxy/tun2proxy-bin"
    chmod +x "$srcdir/static/gfw_proxy/tun2proxy-bin"
    echo "Packaged tun2proxy relay: $TUN2PROXY_BIN"
else
    echo "WARN: tun2proxy-bin not built; the Discord split tunnel will be unavailable."
fi

# Copy built files
cp -r "$srcdir/dist/js" "$pkgdir/opt/vegord/dist/"
cp -r "$srcdir/static" "$pkgdir/opt/vegord/"
cp "$srcdir/package.json" "$pkgdir/opt/vegord/"
cp "$srcdir/LICENSE" "$pkgdir/opt/vegord/"
chmod +x "$pkgdir/opt/vegord/static/gfw_proxy/gfw_proxy" 2>/dev/null || true

# Install icons
cp "$srcdir/build/icon.svg" "$pkgdir/usr/share/icons/hicolor/scalable/apps/vegord.svg"
cp "$srcdir/build/icon.png" "$pkgdir/usr/share/icons/hicolor/256x256/apps/vegord.png" 2>/dev/null || true

# Desktop entry
cat > "$pkgdir/usr/share/applications/vegord-gfw.desktop" <<EOF
[Desktop Entry]
Name=vegord GFW
Comment=Custom Discord desktop app with GFW-resistant proxy
Exec=/opt/vegord/vegord.sh %U
Icon=vegord
Terminal=false
Type=Application
Categories=Network;InstantMessaging;Chat;
MimeType=x-scheme-handler/discord;
StartupWMClass=vegord
Keywords=discord;vegord;electron;chat;
EOF

# Startup script
install -Dm755 /dev/stdin "$pkgdir/opt/vegord/vegord.sh" <<'SCRIPT'
#!/bin/bash
# libgtk4-layer-shell preloaded via LD_PRELOAD crashes Electron's GTK init with
# "gdk_display_manager_get() was called before gtk_init()" (SIGABRT, GTK >= 4.18).
# Strip only that library from LD_PRELOAD and keep any other preloaded entries.
if [ -n "$LD_PRELOAD" ]; then
    LD_PRELOAD="$(printf '%s' "$LD_PRELOAD" | tr ':' '\n' | grep -v 'libgtk4-layer-shell' | paste -sd: -)"
    if [ -n "$LD_PRELOAD" ]; then
        export LD_PRELOAD
    else
        unset LD_PRELOAD
    fi
fi
# --class=vegord matches StartupWMClass in the .desktop file so the
# taskbar/titlebar show the vegord icon instead of the default Electron one
exec /usr/bin/electron --class=vegord /opt/vegord/dist/js/main.js "$@"
SCRIPT

# CLI symlinks
ln -sf /opt/vegord/vegord.sh "$pkgdir/usr/bin/vegord"
ln -sf /opt/vegord/vegord.sh "$pkgdir/usr/bin/vegord-gfw"

# Additional launcher
install -Dm755 /dev/stdin "$pkgdir/usr/bin/vegord-gfw-proxy" <<'SCRIPT'
#!/bin/bash
exec /opt/vegord/vegord.sh "$@"
SCRIPT

# The Rust proxy is the only backend: strip any leftover Python files from the
# staged static dir so the package never ships the removed fallback.
find "$pkgdir/opt/vegord/static" -name '*.py' -delete 2>/dev/null || true
find "$pkgdir/opt/vegord/static" -name '__pycache__' -type d -prune -exec rm -rf {} \; 2>/dev/null || true
find "$pkgdir/opt/vegord/static" -name '*.pyc' -delete 2>/dev/null || true

# Store size in bytes
pkg_size=$(du -sb --apparent-size "$pkgdir" | cut -f1)

# .PKGINFO
cat > "$pkgdir/.PKGINFO" <<EOF
pkgname = ${pkgname}
pkgver = ${pkgver}-${pkgrel}
pkgdesc = vegord - Custom Discord desktop app with built-in high-performance Rust GFW-resistant proxy (SOCKS5/HTTP + TLS Fragment + Multi-DoH + Voice UDP)
url = https://github.com/vergoboy/vegord
builddate = $(date +%s)
packager = vegord Builder
size = ${pkg_size}
arch = x86_64
license = GPL3
depend = electron>=43
depend = libxss
depend = libxtst
depend = glibc
conflict = vesktop-gfw-proxy
provides = vegord-gfw
EOF

# .INSTALL
cat > "$pkgdir/.INSTALL" <<'EOF'
post_install() {
    echo "vegord GFW proxy installed successfully."
    echo "Run 'vegord' or 'vegord-gfw' to start."
    echo "Pass --no-proxy to disable the built-in SOCKS5 proxy."
    # Grant the split-tunnel capability so the Discord TUN tunnel needs no
    # password prompts. Best-effort: skip silently when setcap is unavailable.
    if command -v setcap >/dev/null 2>&1; then
        setcap cap_net_admin,cap_net_raw+ep /opt/vegord/static/gfw_proxy/gfw_proxy 2>/dev/null \
            && echo "setcap: gfw_proxy granted cap_net_admin,cap_net_raw" \
            || echo "setcap: could not grant gfw_proxy capabilities"
        setcap cap_net_admin,cap_net_raw+ep /opt/vegord/static/gfw_proxy/tun2proxy-bin 2>/dev/null \
            && echo "setcap: tun2proxy-bin granted cap_net_admin,cap_net_raw" \
            || echo "setcap: could not grant tun2proxy-bin capabilities"
    else
        echo "setcap not found: Discord split tunnel (discordTunTunnel) will be unavailable."
    fi
}
post_upgrade() { post_install; }
post_remove() {
    echo "vegord GFW proxy has been removed."
}
EOF

# Generate .MTREE (required by pacman >=7 to track files)
echo "Generating .MTREE file..."
mtree_flist=$(mktemp)
find "$pkgdir" -mindepth 1 -maxdepth 1 -name '.*' -prune -o -printf '%P\n' | sort > "$mtree_flist"
bsdtar -czf "$pkgdir/.MTREE" --format=mtree \
  --options='!all,use-set,type,uid,gid,mode,time,size,md5,sha256,link' \
  -C "$pkgdir" -T "$mtree_flist"
rm -f "$mtree_flist"

# Build the package
pkgfile="${pkgname}-${pkgver}-${pkgrel}-x86_64.pkg.tar.zst"
rm -f "/tmp/${pkgfile}" "$srcdir/dist/${pkgfile}"

cd "$pkgdir"
# Create a file list (paths WITHOUT ./ prefix — pacman expects this)
flist=$(mktemp)
printf '.PKGINFO\n.INSTALL\n.MTREE\n' > "$flist"
find . -mindepth 1 -maxdepth 1 ! -name .PKGINFO ! -name .INSTALL ! -name .MTREE | sed 's|^\./||' | sort >> "$flist"
# bsdtar with makepkg-compatible flags: no-fflags, uid/gid 0, no leading ./
bsdtar -c --no-fflags --uid 0 --gid 0 --numeric-owner -f - -T "$flist" | zstd -o "/tmp/${pkgfile}"
rm -f "$flist"

mv "/tmp/${pkgfile}" "$srcdir/dist/"
echo "Package created: dist/${pkgfile}"
ls -lh "$srcdir/dist/${pkgfile}"
