# Maintainer: vegord with GFW proxy
# Build from local source tree. Run: makepkg -si
# NOTE: Run pnpm build first

pkgname=vegord-gfw-proxy
pkgver=1.7.4
pkgrel=1
pkgdesc="vegord - Custom Discord desktop app with built-in GFW-resistant proxy (SOCKS5 + DoH fragment)"
arch=('x86_64' 'aarch64')
url="https://github.com/vergoboy/vegord"
license=('GPL3')
depends=(
    'electron>=43'
    'libxss'
    'libxtst'
    'glibc'
)
optdepends=(
    'pipewire: Linux screenshare with audio'
    'speech-dispatcher: TTS support'
)
install=vegord.install
options=(!purge !strip !zipman)
noextract=()

build() {
    cd "$startdir"
    # Build the GFW-resistant Rust proxy.
    (cd gfw_proxy_rs && cargo build --release --locked) || return 1
    install -Dm755 gfw_proxy_rs/target/release/gfw_proxy static/gfw_proxy/gfw_proxy
    # Build the tun2proxy relay (Discord split tunnel). Optional: the package
    # still installs without it, only the discordTunTunnel feature is lost.
    if [ -d tun2proxy ]; then
        (cd tun2proxy && cargo build --release --bin tun2proxy-bin --locked) || true
        install -Dm755 tun2proxy/target/release/tun2proxy-bin static/gfw_proxy/tun2proxy-bin 2>/dev/null || true
    fi
}

package() {
    cd "$startdir"

    install -dm755 "${pkgdir}/opt/vegord/dist"
    cp -r dist/js "${pkgdir}/opt/vegord/dist/"
    cp -r static package.json LICENSE "${pkgdir}/opt/vegord/"

    install -Dm644 build/icon.svg "${pkgdir}/usr/share/icons/hicolor/scalable/apps/vegord.svg"
    install -Dm644 build/icon-48.png "${pkgdir}/usr/share/icons/hicolor/48x48/apps/vegord.png" 2>/dev/null || true
    install -Dm644 build/icon.png "${pkgdir}/usr/share/icons/hicolor/256x256/apps/vegord.png" 2>/dev/null || true
    install -Dm644 build/icon-512.png "${pkgdir}/usr/share/icons/hicolor/512x512/apps/vegord.png" 2>/dev/null || true

    cp build/icon.png "${pkgdir}/opt/vegord/static/icon.png"

    install -Dm644 /dev/stdin "${pkgdir}/usr/share/applications/vegord.desktop" <<EOF
[Desktop Entry]
Name=vegord
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

    install -Dm644 /dev/stdin "${pkgdir}/usr/share/applications/vegord-gfw.desktop" <<EOF
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

    install -Dm755 /dev/stdin "${pkgdir}/opt/vegord/vegord.sh" <<'SCRIPT'
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
exec /usr/bin/electron --class=vegord /opt/vegord/dist/js/main.js "$@"
SCRIPT

    install -dm755 "${pkgdir}/usr/bin"
    ln -sf /opt/vegord/vegord.sh "${pkgdir}/usr/bin/vegord"
    ln -sf /opt/vegord/vegord.sh "${pkgdir}/usr/bin/vegord-gfw"

    install -Dm755 /dev/stdin "${pkgdir}/usr/bin/vegord-gfw-proxy" <<'SCRIPT'
#!/bin/bash
exec /opt/vegord/vegord.sh "$@"
SCRIPT

    # The Rust proxy is the only backend: strip any leftover Python files from
    # the staged static dir so the package never ships the removed fallback.
    find "${pkgdir}/opt/vegord/static" -name '*.py' -delete 2>/dev/null || true
    find "${pkgdir}/opt/vegord/static" -name '__pycache__' -type d -prune -exec rm -rf {} \; 2>/dev/null || true
    find "${pkgdir}/opt/vegord/static" -name '*.pyc' -delete 2>/dev/null || true
    install -dm755 "${pkgdir}/opt/vegord/static/gfw_proxy/logs"
}
