# Maintainer: Vesktop with GFW proxy
# Build from local source tree. Run: makepkg -si

pkgname=vesktop-gfw-proxy
pkgver=1.6.5
pkgrel=2
pkgdesc="Vesktop - Custom Discord desktop app with built-in GFW-resistant proxy (SOCKS5 + DoH fragment)"
arch=('x86_64' 'aarch64')
url="https://github.com/Vencord/Vesktop"
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
makedepends=('nodejs>=22' 'pnpm>=11' 'git' 'python' 'python-dnspython' 'python-requests')
optdepends=(
    'pipewire: Linux screenshare with audio'
    'speech-dispatcher: TTS support'
)
source=("${pkgname}::git+file://${PWD}")
sha256sums=('SKIP')

prepare() {
    cd "${srcdir}/${pkgname}"
    # GFW proxy files are untracked in git, so copy from the original project
    cp -r "$startdir/gfw_resist_HTTPS_proxy" .
    pnpm install
}

build() {
    cd "${srcdir}/${pkgname}"
    pnpm build
}

package() {
    cd "${srcdir}/${pkgname}"

    # Ensure proxy files exist (build.mts copies them, but fallback if it failed)
    if [ ! -d "static/gfw_proxy" ]; then
        cp -r "$startdir/gfw_resist_HTTPS_proxy"/. static/gfw_proxy/
    fi

    install -dm755 "${pkgdir}/opt/vesktop/dist"
    cp -r dist/js "${pkgdir}/opt/vesktop/dist/"
    cp -r static package.json LICENSE "${pkgdir}/opt/vesktop/"

    # Install icons
    install -Dm644 build/icon.svg "${pkgdir}/usr/share/icons/hicolor/scalable/apps/vesktop-gfw.svg"
    install -Dm644 build/icon.png "${pkgdir}/usr/share/icons/hicolor/256x256/apps/vesktop-gfw.png" 2>/dev/null || true

    # Desktop entry
    install -Dm644 /dev/stdin "${pkgdir}/usr/share/applications/vesktop-gfw.desktop" <<EOF
[Desktop Entry]
Name=Vesktop GFW
Comment=Custom Discord desktop app with GFW-resistant proxy
Exec=/opt/vesktop/vesktop.sh %U
Icon=vesktop-gfw
Terminal=false
Type=Application
Categories=Network;InstantMessaging;Chat;
MimeType=x-scheme-handler/discord;
StartupWMClass=vesktop
Keywords=discord;vencord;electron;chat;
EOF

    # Startup script - launches electron with the main app bundle
    install -Dm755 /dev/stdin "${pkgdir}/opt/vesktop/vesktop.sh" <<EOF
#!/bin/bash
# Vesktop launcher with GFW-resistant proxy support
# Pass --no-proxy to disable the built-in SOCKS5 proxy
exec /usr/bin/electron /opt/vesktop/dist/js/main.js "\$@"
EOF

    # Symlink for CLI (unique name to avoid conflict with original vesktop AUR package)
    install -dm755 "${pkgdir}/usr/bin"
    ln -sf /opt/vesktop/vesktop.sh "${pkgdir}/usr/bin/vesktop-gfw"

    # Additional launcher aliases
    install -Dm755 /dev/stdin "${pkgdir}/usr/bin/vesktop-gfw-proxy" <<EOF
#!/bin/bash
exec /opt/vesktop/vesktop.sh "\$@"
EOF
    ln -sf /opt/vesktop/vesktop.sh "${pkgdir}/usr/bin/vesktop-gfw-original"

    # Clean up any __pycache__ or .pyc files from the proxy directory
    find "${pkgdir}/opt/vesktop/static" -name '__pycache__' -type d -prune -exec rm -rf {} \; 2>/dev/null || true
    find "${pkgdir}/opt/vesktop/static" -name '*.pyc' -delete 2>/dev/null || true

    # Create log directory for proxy
    install -dm755 "${pkgdir}/opt/vesktop/static/gfw_proxy/logs"
}
