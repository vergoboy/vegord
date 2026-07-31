#!/bin/bash
set -e
# Build an Arch Linux package from the current build directory
# Uses the same approach as makepkg for compatibility

pkgname=vegord-gfw-proxy
pkgver=1.6.6
pkgrel=2
pkgdir="/tmp/${pkgname}-pkg"
srcdir="$(cd "$(dirname "$0")" && pwd)"

rm -rf "$pkgdir"
mkdir -p "$pkgdir/opt/vegord/dist"
mkdir -p "$pkgdir/usr/share/icons/hicolor/scalable/apps"
mkdir -p "$pkgdir/usr/share/icons/hicolor/256x256/apps"
mkdir -p "$pkgdir/usr/share/applications"
mkdir -p "$pkgdir/usr/bin"
mkdir -p "$pkgdir/opt/vegord/static/gfw_proxy/logs"

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

# Startup script
install -Dm755 /dev/stdin "$pkgdir/opt/vegord/vegord.sh" <<'SCRIPT'
#!/bin/bash
# Unset LD_PRELOAD to avoid GDK/GTK crashes (e.g. libgtk4-layer-shell.so)
unset LD_PRELOAD
# --class=vegord matches StartupWMClass in the .desktop file so the
# taskbar/titlebar show the Vegcord icon instead of the default Electron one
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

# Clean pycache
find "$pkgdir/opt/vegord/static" -name '__pycache__' -type d -prune -exec rm -rf {} \; 2>/dev/null || true
find "$pkgdir/opt/vegord/static" -name '*.pyc' -delete 2>/dev/null || true

# Store size in bytes
pkg_size=$(du -sb --apparent-size "$pkgdir" | cut -f1)

# .PKGINFO
cat > "$pkgdir/.PKGINFO" <<EOF
pkgname = ${pkgname}
pkgver = ${pkgver}-${pkgrel}
pkgdesc = Vegcord - Custom Discord desktop app with built-in high-performance Rust GFW-resistant proxy (SOCKS5/HTTP + TLS Fragment + Multi-DoH + Voice UDP)
url = https://github.com/vergoboy/Vegcord
builddate = $(date +%s)
packager = Vegcord Builder
size = ${pkg_size}
arch = x86_64
license = GPL3
depend = electron>=43
depend = libxss
depend = libxtst
depend = glibc
optdepend = python
optdepend = python-dnspython
optdepend = python-requests
conflict = vesktop-gfw-proxy
provides = vegord-gfw
EOF

# .INSTALL
cat > "$pkgdir/.INSTALL" <<'EOF'
post_install() {
    echo "Vegcord GFW proxy installed successfully."
    echo "Run 'vegord' or 'vegord-gfw' to start."
    echo "Pass --no-proxy to disable the built-in SOCKS5 proxy."
}
post_upgrade() { post_install; }
post_remove() {
    echo "Vegcord GFW proxy has been removed."
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
