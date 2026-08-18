#!/bin/bash
set -e

# Simple AppImage build that ensures environment variables are set

VERSION=${1:-latest}
ARCH=${2:-x86_64}
APPDIR="StrikeHub.AppDir"

# Derive the Rust target triple from ARCH
case "$ARCH" in
  x86_64)  TARGET="x86_64-unknown-linux-gnu" ;;
  aarch64) TARGET="aarch64-unknown-linux-gnu" ;;
  *)       echo "Unsupported arch: $ARCH"; exit 1 ;;
esac

echo "Building StrikeHub AppImage ($ARCH) with working env vars..."

# Clean up
rm -rf "$APPDIR"
rm -f StrikeHub*.AppImage

# Download appimagetool if needed
APPIMAGETOOL="appimagetool-${ARCH}.AppImage"
if [ ! -f "$APPIMAGETOOL" ]; then
    echo "Downloading $APPIMAGETOOL..."
    wget -q "https://github.com/AppImage/AppImageKit/releases/download/continuous/${APPIMAGETOOL}"
    chmod +x "$APPIMAGETOOL"
fi

# Create AppDir structure
mkdir -p "$APPDIR/usr/bin"
mkdir -p "$APPDIR/usr/share/applications"
mkdir -p "$APPDIR/usr/share/icons/hicolor/scalable/apps"

# Copy binaries (BIN_DIR can be overridden for debug builds)
BIN_DIR="${BIN_DIR:-target/${TARGET}/release}"
echo "Copying binaries from ${BIN_DIR}..."
cp "${BIN_DIR}/strikehub" "$APPDIR/usr/bin/strikehub-real"
chmod +x "$APPDIR/usr/bin/strikehub-real"

# Create wrapper script that sets env vars
cat > "$APPDIR/usr/bin/strikehub" << 'EOF'
#!/bin/bash
HERE="$(dirname "$(readlink -f "${0}")")"

# Set default Strike48 URLs if not already set
if [ -z "$STRIKE48_API_URL" ]; then
    export STRIKE48_API_URL="https://studio.strike48.com"
fi
if [ -z "$STRIKE48_URL" ]; then
    export STRIKE48_URL="wss://studio.strike48.com"
fi

# Force X11 backend to avoid Wayland protocol errors with WebKitGTK
export GDK_BACKEND=x11

# Execute the real binary
exec "$HERE/strikehub-real" "$@"
EOF
chmod +x "$APPDIR/usr/bin/strikehub"

# Copy connectors if they exist
if [ -f "dist/ks-connector" ]; then
    echo "✓ Adding ks-connector..."
    cp "dist/ks-connector" "$APPDIR/usr/bin/"
    chmod +x "$APPDIR/usr/bin/ks-connector"
fi

if [ -f "dist/pentest-agent" ]; then
    echo "✓ Adding pentest-agent..."
    cp "dist/pentest-agent" "$APPDIR/usr/bin/"
    chmod +x "$APPDIR/usr/bin/pentest-agent"
fi

# Copy desktop file and icon
cp "assets/strikehub.desktop" "$APPDIR/usr/share/applications/"
cp "assets/strikehub.desktop" "$APPDIR/"
cp "crates/sh-ui/src/assets/icons/strike48-mark.svg" "$APPDIR/usr/share/icons/hicolor/scalable/apps/strikehub.svg"

# Generate PNG icons from SVG for desktop environment compatibility.
# Many DEs (GNOME, KDE, XFCE) prefer PNG over SVG for app icons.
SVG_ICON="crates/sh-ui/src/assets/icons/strike48-mark.svg"
for SIZE in 256 128 64 48 32 16; do
    DIR="$APPDIR/usr/share/icons/hicolor/${SIZE}x${SIZE}/apps"
    mkdir -p "$DIR"
    if command -v rsvg-convert &> /dev/null; then
        rsvg-convert -w "$SIZE" -h "$SIZE" "$SVG_ICON" -o "$DIR/strikehub.png"
    else
        convert -background none "$SVG_ICON" -resize "${SIZE}x${SIZE}" "$DIR/strikehub.png"
    fi
done

# AppImage requires a PNG icon at the AppDir root and a .DirIcon for file managers.
cp "$APPDIR/usr/share/icons/hicolor/256x256/apps/strikehub.png" "$APPDIR/strikehub.png"
cp "$APPDIR/usr/share/icons/hicolor/256x256/apps/strikehub.png" "$APPDIR/.DirIcon"

# ── Portable packaging via linuxdeploy + GTK plugin ──────────────────────
# linuxdeploy walks each --executable's ldd closure, copies the needed .so files
# into usr/lib, patches their rpaths to $ORIGIN/../lib, and (via the gtk plugin)
# bundles the GTK runtime pieces a bare ldd copy misses: gdk-pixbuf loaders, GIO
# modules, gsettings schemas. It does NOT bundle WebKit's out-of-process helper
# binaries (WebKitNetworkProcess/WebKitWebProcess) — those are copied separately
# in Phase 2. glibc/libGL/X11 are intentionally NOT bundled (linuxdeploy's
# excludelist) — they come from the host, so this must build on the oldest glibc
# we support. Net effect: the AppImage runs on a distro WITHOUT webkit2gtk
# installed (previously it shipped zero libs and relied on the host).
LINUXDEPLOY="linuxdeploy-${ARCH}.AppImage"
LINUXDEPLOY_GTK="linuxdeploy-plugin-gtk.sh"
if [ ! -f "$LINUXDEPLOY" ]; then
    echo "Downloading $LINUXDEPLOY..."
    wget -q "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/${LINUXDEPLOY}"
    chmod +x "$LINUXDEPLOY"
fi
if [ ! -f "$LINUXDEPLOY_GTK" ]; then
    echo "Downloading $LINUXDEPLOY_GTK..."
    wget -q "https://raw.githubusercontent.com/linuxdeploy/linuxdeploy-plugin-gtk/master/${LINUXDEPLOY_GTK}"
    chmod +x "$LINUXDEPLOY_GTK"
fi

# linuxdeploy's `--output appimage` shells out to `appimagetool` on PATH — make
# the arch-suffixed download we already have discoverable under that bare name.
mkdir -p .ldbin
ln -sf "$PWD/${APPIMAGETOOL}" ".ldbin/appimagetool"
ln -sf "$PWD/${LINUXDEPLOY_GTK}" ".ldbin/linuxdeploy-plugin-gtk.sh"
export PATH="$PWD/.ldbin:$PATH"

# Run linuxdeploy's own AppImages by extraction (CI runners lack a usable FUSE
# in some images) and tell the gtk plugin we target GTK3, not GTK4.
export APPIMAGE_EXTRACT_AND_RUN=1
export DEPLOY_GTK_VERSION=3

# Env + CWD as an AppRun hook. linuxdeploy's generated AppRun sources
# apprun-hooks/*.sh before exec, so these apply whether it launches the
# `strikehub` wrapper or `strikehub-real` directly (version-dependent).
#  - GDK_BACKEND=x11 avoids the WebKitGTK Wayland surface bug.
#  - LD_LIBRARY_PATH makes WebKit's freshly-spawned helper processes (which don't
#    inherit the main binary's patched rpath) resolve the bundled libwebkit/gtk.
#  - cd "$APPDIR": WebKitGTK 2.52 removed WEBKIT_EXEC_PATH and hardcodes an
#    ABSOLUTE libexec path into libwebkit; Phase 2 below binary-patches that to a
#    RELATIVE path, which g_subprocess resolves against CWD — so anchor CWD at the
#    AppDir root. StrikeHub uses absolute paths (data dir, current_exe-based
#    connector resolution), so changing CWD here is safe.
mkdir -p "$APPDIR/apprun-hooks"
cat > "$APPDIR/apprun-hooks/00-strike48-env.sh" << 'HOOKEOF'
if [ -z "$STRIKE48_API_URL" ]; then export STRIKE48_API_URL="https://studio.strike48.com"; fi
if [ -z "$STRIKE48_URL" ]; then export STRIKE48_URL="wss://studio.strike48.com"; fi
export GDK_BACKEND=x11
export LD_LIBRARY_PATH="${APPDIR}/usr/lib:${LD_LIBRARY_PATH}"
cd "${APPDIR}" 2>/dev/null || true
HOOKEOF

# Phase 1 — deploy libs + GTK runtime into the AppDir (no packaging yet). The
# connectors ride along as extra executables so their ldd closures bundle while
# staying usr/bin siblings (the resolver + newest-wins seed depend on that);
# --executable deploys their libs, it does not relocate them.
EXTRA_EXE=()
[ -f "$APPDIR/usr/bin/pentest-agent" ] && EXTRA_EXE+=(--executable "$APPDIR/usr/bin/pentest-agent")
[ -f "$APPDIR/usr/bin/ks-connector" ]  && EXTRA_EXE+=(--executable "$APPDIR/usr/bin/ks-connector")
echo "Deploying libraries via linuxdeploy..."
"./${LINUXDEPLOY}" \
    --appdir "$APPDIR" \
    --executable "$APPDIR/usr/bin/strikehub-real" \
    "${EXTRA_EXE[@]}" \
    --desktop-file "$APPDIR/usr/share/applications/strikehub.desktop" \
    --icon-file "$APPDIR/strikehub.png" \
    --plugin gtk

# Phase 2 — bundle WebKit's out-of-process helpers AND make them relocatable.
# The gtk plugin bundles GTK modules but NOT WebKitNetworkProcess/WebKitWebProcess
# (spawned at runtime, invisible to ldd). Worse, WebKitGTK 2.52 hardcodes the
# ABSOLUTE PKGLIBEXECDIR (/usr/lib/<triplet>/webkit2gtk-4.1) into libwebkit and
# removed the WEBKIT_EXEC_PATH override, so it spawns the helpers from that fixed
# path — which doesn't exist on a host without webkit. Fix: (a) bundle the helper
# dir at the SAME relative multiarch path, (b) binary-patch the compiled absolute
# path to a same-length RELATIVE one (drop leading '/', pad with NUL), (c) the
# AppRun hook cd's to $APPDIR so g_subprocess resolves the relative path in-bundle.
WK_HELPER="$(find /usr/lib /usr/libexec -name WebKitNetworkProcess -path '*webkit2gtk-4.1*' 2>/dev/null | head -1)"
if [ -z "$WK_HELPER" ]; then
    echo "ERROR: WebKitNetworkProcess not found on host — cannot build a portable AppImage" >&2
    exit 1
fi
WK_SRC="$(dirname "$WK_HELPER")"   # /usr/lib/<triplet>/webkit2gtk-4.1
WK_REL="${WK_SRC#/}"                # usr/lib/<triplet>/webkit2gtk-4.1
mkdir -p "$APPDIR/$WK_REL"
cp -rL "$WK_SRC/." "$APPDIR/$WK_REL/"
chmod +x "$APPDIR/$WK_REL/WebKitNetworkProcess" "$APPDIR/$WK_REL/WebKitWebProcess" 2>/dev/null || true
echo "Bundled WebKit helpers: $WK_SRC -> $WK_REL"

LIBWK="$(find "$APPDIR/usr/lib" -maxdepth 1 -name 'libwebkit2gtk-4.1.so.0*' -type f 2>/dev/null | head -1)"
if [ -z "$LIBWK" ]; then
    echo "ERROR: bundled libwebkit2gtk-4.1.so.0 not found (linuxdeploy should have deployed it)" >&2
    exit 1
fi
echo "Patching hardcoded WebKit exec path in $(basename "$LIBWK")..."
python3 - "$LIBWK" "$WK_SRC" <<'PY'
import sys
lib, wk_src = sys.argv[1], sys.argv[2]
data = bytearray(open(lib, 'rb').read())
def patch(absdir):
    old = absdir.encode() + b'\x00'            # "/usr/lib/.../webkit2gtk-4.1\0"
    rel = absdir[1:].encode() + b'\x00\x00'    # "usr/lib/.../webkit2gtk-4.1\0\0" (same length)
    assert len(old) == len(rel), (len(old), len(rel))
    n = data.count(old)
    if n:
        data[:] = data.replace(old, rel)
    return n
n_dir = patch(wk_src)                          # the helper libexec dir
n_ib  = patch(wk_src + "/injected-bundle/")    # the injected-bundle dir
open(lib, 'wb').write(data)
print(f"  patched: exec-dir x{n_dir}, injected-bundle x{n_ib}")
if n_dir == 0:
    sys.exit("ERROR: hardcoded WebKit exec path not found in libwebkit — patch failed")
PY

# Phase 3 — package the fully-populated AppDir (linuxdeploy already wrote AppRun).
echo "Packaging AppImage..."
ARCH=$ARCH "./${APPIMAGETOOL}" "$APPDIR" "StrikeHub-${VERSION}-${ARCH}.AppImage"

echo ""
echo "✅ Portable AppImage created: StrikeHub-${VERSION}-${ARCH}.AppImage"
echo "   (GTK/WebKit runtime + out-of-process helpers bundled — runs without host webkit2gtk)"
