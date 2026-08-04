# Maintainer: Vegcord with GFW proxy
# Build from local source tree. Run: makepkg -si
# NOTE: Run pnpm build first

pkgname=vegord-gfw-proxy
pkgver=1.6.11
pkgrel=1
pkgdesc="Vegcord - Custom Discord desktop app with built-in GFW-resistant proxy (SOCKS5 + DoH fragment)"
arch=('x86_64' 'aarch64')
url="https://github.com/vergoboy/Vegcord"
license=('GPL3')
depends=(
    'electron>=43'
    'python'
    'python-dnspython'
    'python-requests'
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
Name=Vegcord
Comment=Custom Discord desktop app with GFW-resistant proxy
Exec=/opt/vegord/vegord.sh %U
Icon=vegord
Terminal=false
Type=Application
Categories=Network;InstantMessaging;Chat;
MimeType=x-scheme-handler/discord;
StartupWMClass=vegord
Keywords=discord;vencord;electron;chat;
EOF

    install -Dm644 /dev/stdin "${pkgdir}/usr/share/applications/vegord-gfw.desktop" <<EOF
[Desktop Entry]
Name=Vegcord GFW
Comment=Custom Discord desktop app with GFW-resistant proxy
Exec=/opt/vegord/vegord.sh %U
Icon=vegord
Terminal=false
Type=Application
Categories=Network;InstantMessaging;Chat;
MimeType=x-scheme-handler/discord;
StartupWMClass=vegord
Keywords=discord;vencord;electron;chat;
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

    find "${pkgdir}/opt/vegord/static" -name '__pycache__' -type d -prune -exec rm -rf {} \; 2>/dev/null || true
    find "${pkgdir}/opt/vegord/static" -name '*.pyc' -delete 2>/dev/null || true
    install -dm755 "${pkgdir}/opt/vegord/static/gfw_proxy/logs"
}
