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

# ── Pinned AppImage tooling (Strike48/project-management#382) ───────────────
# The AppImage's EMBEDDED RUNTIME comes from the tool that packages the
# AppDir. This script previously downloaded AppImageKit's "continuous"
# appimagetool, which embeds the legacy type-2 runtime: it resolves FUSE 2 at
# launch (dlopen libfuse.so.2) and fails on stock Ubuntu 24.04+, which only
# ships FUSE 3 — and silently, when launched from the app grid (the error
# only reaches the journal). See Strike48/project-management#377 finding 2
# (BLOCKER) and #382.
#
# Fix: pin BOTH the tool and the runtime it embeds, by release tag AND
# sha256 (the same pin-and-verify style this repo uses for its connectors):
#   * appimagetool 1.9.1 — the AppImage/appimagetool successor to the
#     unmaintained AppImageKit releases. Its own embedded runtime is already
#     the static FUSE-3 one, and it accepts a pinned runtime file, so the
#     runtime embedded in OUR AppImages never drifts from the pin below.
#   * type2-runtime 20251108 — github.com/AppImage/type2-runtime, the
#     official static type-2 runtime: built -static -lfuse3 (Alpine/musl), so
#     the AppImage needs NO host libfuse at all — only fusermount3, which
#     stock Ubuntu 24.04+ provides (the runtime locates fusermount* on $PATH;
#     without any fusermount, APPIMAGE_EXTRACT_AND_RUN=1 still works).
# Bump deliberately: when moving either pin, update the sha256 from the
# release asset's digest (shown on the GitHub release page / API).
APPIMAGETOOL_VERSION="1.9.1"
TYPE2_RUNTIME_VERSION="20251108"
case "$ARCH" in
  x86_64)
    APPIMAGETOOL_SHA256="ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0"
    TYPE2_RUNTIME_SHA256="2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d"
    ;;
  aarch64)
    APPIMAGETOOL_SHA256="f0837e7448a0c1e4e650a93bb3e85802546e60654ef287576f46c71c126a9158"
    TYPE2_RUNTIME_SHA256="00cbdfcf917cc6c0ff6d3347d59e0ca1f7f45a6df1a428a0d6d8a78664d87444"
    ;;
  *)
    echo "Unsupported arch: $ARCH (no pinned AppImage tooling)"; exit 1 ;;
esac

# Download a pinned release artifact and verify its sha256 before use. The
# old script wget'd AppImageKit's moving "continuous" assets with no checksum
# at all; both pins above are now verified.
download_pinned() {
    local url="$1" dest="$2" want="$3" got
    echo "Downloading $dest (pinned) ..."
    wget -q "$url" -O "$dest" || { echo "ERROR: download failed: $url" >&2; exit 1; }
    got="$(sha256sum "$dest" | awk '{print $1}')"
    if [ "$got" != "$want" ]; then
        echo "ERROR: sha256 mismatch for $dest" >&2
        echo "  expected: $want" >&2
        echo "  got:      $got" >&2
        echo "  (release asset changed or was tampered with — do not proceed)" >&2
        exit 1
    fi
    echo "  sha256 verified: $got"
}

echo "Building StrikeHub AppImage ($ARCH) with working env vars..."

# Clean up
rm -rf "$APPDIR"
rm -f StrikeHub*.AppImage

# appimagetool (pinned; see above). It packages the AppDir and embeds a
# type-2 runtime — that runtime is what decides which FUSE the AppImage
# needs, which is why both the tool and the runtime are pinned.
APPIMAGETOOL="appimagetool-${ARCH}.AppImage"
if [ ! -f "$APPIMAGETOOL" ]; then
    download_pinned "https://github.com/AppImage/appimagetool/releases/download/${APPIMAGETOOL_VERSION}/${APPIMAGETOOL}" "$APPIMAGETOOL" "$APPIMAGETOOL_SHA256"
    chmod +x "$APPIMAGETOOL"
fi

# The static FUSE-3 type-2 runtime embedded into OUR AppImage (pinned; see
# above). Passed to appimagetool below via --runtime-file, and exported as
# APPIMAGETOOL_RUNTIME_FILE for any appimagetool linuxdeploy may spawn, so
# the embedded runtime is exactly this file — never a downloaded default.
TYPE2_RUNTIME="type2-runtime-${ARCH}"
if [ ! -f "$TYPE2_RUNTIME" ]; then
    download_pinned "https://github.com/AppImage/type2-runtime/releases/download/${TYPE2_RUNTIME_VERSION}/runtime-${ARCH}" "$TYPE2_RUNTIME" "$TYPE2_RUNTIME_SHA256"
fi

# Create AppDir structure
mkdir -p "$APPDIR/usr/bin"
mkdir -p "$APPDIR/usr/share/applications"
mkdir -p "$APPDIR/usr/share/icons/hicolor/scalable/apps"

# Copy binaries (BIN_DIR can be overridden for debug builds)
BIN_DIR="${BIN_DIR:-target/${TARGET}/release}"
echo "Copying binaries from ${BIN_DIR}..."
# The payload keeps the end-user name `strikehub` (no `-real` suffix): on
# X11 the toolkit derives WM_CLASS from the process name (argv[0]), and
# GNOME matches the window to strikehub.desktop via that class. The old
# `strikehub-real` name made WM_CLASS `strikehub-real`, which matched no
# desktop entry -> generic gear icon (project-management#380, 377 finding 6).
cp "${BIN_DIR}/strikehub" "$APPDIR/usr/bin/strikehub"
chmod +x "$APPDIR/usr/bin/strikehub"

# NOTE: no separate env-var wrapper script is needed here. The AppRun that
# linuxdeploy generates (see below) sources apprun-hooks/*.sh and exports the
# same STRIKE48_API_URL / STRIKE48_URL / GDK_BACKEND defaults, so the real
# binary can be the payload itself — which is what keeps argv[0] (and thus
# WM_CLASS) equal to `strikehub`.

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
# in Phase 2. glibc/libstdc++ are intentionally NOT bundled (linuxdeploy's
# excludelist) — they come from the host, so this must build on the oldest
# glibc we support (the floor is enforced by scripts/check-glibc-floor.sh in
# CI). The GTK/WebKit runtime is bundled by linuxdeploy + Phase 2; the
# graphics stack (wayland + EGL + GL + glvnd + mesa DRI) is bundled by Phase
# 2d so STOCK hosts with no graphics packages at all (minimal/server images)
# run out of the box. Net effect: the AppImage runs on a distro WITHOUT
# webkit2gtk or mesa installed (it once shipped zero libs and relied on the
# host).
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
# Any appimagetool invoked below (directly, or by linuxdeploy) must embed the
# pinned static FUSE-3 runtime — never a downloaded default (#382).
export APPIMAGETOOL_RUNTIME_FILE="$PWD/${TYPE2_RUNTIME}"

# The build host's multiarch lib dir (e.g. /usr/lib/x86_64-linux-gnu). Needed
# here for the hook's in-bundle DRI path (below) and by Phase 2d (HOST_LIBDIR).
HOST_TRIPLET_DIR="$(find /usr/lib -maxdepth 1 -type d -name '*-linux-gnu' 2>/dev/null | head -1)"
[ -n "$HOST_TRIPLET_DIR" ] || { echo "ERROR: no multiarch lib dir under /usr/lib — cannot determine the host triplet" >&2; exit 1; }
TRIPLET_LIBREL="${HOST_TRIPLET_DIR#/}"

# Env + CWD as an AppRun hook. linuxdeploy's generated AppRun sources
# apprun-hooks/*.sh in ALPHABETICAL order before exec'ing the app, and it only
# exports APPDIR inside the linuxdeploy-plugin-gtk.sh hook — which sorts AFTER
# any 00-* file. So this hook must compute APPDIR itself. (Relying on the
# inherited variable was a real bug: the hook ran with APPDIR EMPTY, the
# LD_LIBRARY_PATH line degenerated to "/usr/lib:", and the cd anchor below was
# a no-op. Same bug class as pick's sub-test-90: an env-init hook that runs
# before the variable it assumes is set. The derivation below also uses bash
# builtins only — no dirname/readlink — so a poisoned environment cannot break
# it.)
#  - No LD_LIBRARY_PATH: the bundle's flat usr/lib holds build-host copies of
#    system-ish libs (libsystemd 249, icu70, ...). Putting it on the global
#    LD_LIBRARY_PATH makes it shadow the HOST's copies for every child process
#    and breaks symbol versioning on newer distros — verified on ubuntu 26.04:
#    tail/cut/readlink die with "version `LIBSYSTEMD_254' not found" and the app
#    fails to exec. The main binary and the connectors already find the bundle
#    via their patched $ORIGIN rpaths, and the WebKit helpers get the bundled
#    libwebkit via the rpath baked into them in Phase 2b — neither needs
#    LD_LIBRARY_PATH.
#  - GDK_BACKEND=x11 avoids the WebKitGTK Wayland surface bug.
#  - cd "$APPDIR": WebKitGTK 2.5x hardcodes the helper libexec dir into
#    libwebkit; Phase 2 below binary-patches that ABSOLUTE path to a
#    same-length RELATIVE one, which the kernel resolves against the CWD at
#    spawn. Without this anchor the helpers are not found at all when the
#    AppImage is launched from a normal CWD — verified on ubuntu 26.04: g_error
#    abort "Failed to spawn child process 'usr/lib/.../WebKitNetworkProcess'
#    (No such file or directory)". StrikeHub uses absolute paths (data dir,
#    current_exe-based connector resolution), so changing CWD here is safe.
mkdir -p "$APPDIR/apprun-hooks"
cat > "$APPDIR/apprun-hooks/00-strike48-env.sh" << HOOKEOF
# Derive APPDIR from THIS hook's own location: the generated AppRun sources
# apprun-hooks/*.sh BEFORE anything exports APPDIR, so never assume it is set
# (it ran empty once — the cd anchor below then no-oped and the WebKit helpers
# could not spawn). Pure bash builtins: no dirname/readlink, so a poisoned env
# (e.g. a host whose coreutils links libsystemd) cannot abort the launch.
# This is an UNQUOTED heredoc: every $ that must survive to runtime is
# \$-escaped. Without the escapes, the build-time expansion baked the build
# host's own script path into src and matched the case against an empty
# string, so the derivation below silently no-oped.
src="\${BASH_SOURCE[0]:-\$0}"
case "\$src" in
    */*) APPDIR="\${APPDIR:-\$(cd "\${src%/*}/.." 2>/dev/null && pwd -P || true)}" ;;
esac
[ -n "\${APPDIR:-}" ] || APPDIR="\$PWD"
export APPDIR
# Default-only: the \$ escapes make these evaluate at RUNTIME. Unescaped, the
# heredoc expands them at build time (the vars are unset there) and the
# generated hook clobbers a user's pre-launch export. User-set values win.
: "\${STRIKE48_API_URL:=https://studio.strike48.com}"
export STRIKE48_API_URL
: "\${STRIKE48_URL:=wss://studio.strike48.com}"
export STRIKE48_URL
export GDK_BACKEND=x11
# WebKit's WebProcess (EGL/gbm init + IPC sockets) requires XDG_RUNTIME_DIR;
# without it EGL init aborts ("Could not create default EGL display:
# EGL_BAD_PARAMETER") and the window never paints. Real desktop sessions
# always have it — create a private one for headless runs. User-set wins.
if [ -z "\${XDG_RUNTIME_DIR:-}" ]; then
    XDG_RUNTIME_DIR="\${TMPDIR:-/tmp}/xdg-runtime"
    mkdir -p "\$XDG_RUNTIME_DIR" 2>/dev/null || true
    # 0755, not the spec-mandated 0700: WebKit's sandboxed WebProcess is uid-
    # remapped and could not enter a 0700 root-owned dir (EGL init died with
    # EGL_BAD_PARAMETER and the window never painted). Fine for a fallback dir
    # under /tmp; real desktop sessions keep their own 0700 dir untouched.
    chmod 755 "\$XDG_RUNTIME_DIR" 2>/dev/null || true
    export XDG_RUNTIME_DIR
fi
# WebKit's dmabuf/EGL-GBM renderer needs a host GPU with userspace that
# matches the *bundled* mesa (e.g. NVIDIA GPUs have none on a stock machine)
# — exactly the pairing a stock install cannot guarantee; it dies with
# "Could not create default EGL display: EGL_BAD_PARAMETER". Force the GLX
# path instead: glvnd dispatches into the bundled mesa (vendor modules + DRI
# drivers from this AppImage), falling back to llvmpipe where the host has no
# matching open GPU driver. User-set values win.
: "\${WEBKIT_DISABLE_DMABUF_RENDERER:=1}"
export WEBKIT_DISABLE_DMABUF_RENDERER
# The graphics runtime is bundled (Phase 2d): mesa dlopens its DRI drivers
# from LIBGL_DRIVERS_PATH and glvnd dlopens the vendor modules (libGLX_mesa/
# libEGL_mesa, which live in \$APPDIR/usr/lib) from __GLX_DRIVERS_PATH. Point
# both at the in-bundle copies so a stock host with no graphics stack can
# software-render. User-set values win.
: "\${__GLX_DRIVERS_PATH:=\$APPDIR/usr/lib}"
export __GLX_DRIVERS_PATH
: "\${LIBGL_DRIVERS_PATH:=\$APPDIR/${TRIPLET_LIBREL}/dri}"
export LIBGL_DRIVERS_PATH
cd "\${APPDIR}" 2>/dev/null || true
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
    --executable "$APPDIR/usr/bin/strikehub" \
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

# Phase 2b — bake an rpath into the bundled WebKit helper binaries.
# The helper ELFs ship WITHOUT any rpath, so at spawn their dynamic linker
# resolves libwebkit2gtk-4.1.so.0 from LD_LIBRARY_PATH (absent — WebKit spawns
# helpers with a sanitized env) and then the HOST's loader cache. Both failure
# modes were reproduced in containers against this packaging:
#   - host has a DIFFERENT webkit (ubuntu 26.04 ships 2.52.6, the bundle was
#     built against 2.50.4): the helper linked the host lib while the UI
#     process ran the bundled one -> IPC version mismatch, the web process
#     never became ready, the sign-in UI never mapped (startup hang, 4/4).
#   - host has NO webkit (pristine ubuntu 22.04): the helper's exec failed in
#     the loader — "error while loading shared libraries: libwebkit2gtk-4.1.so.0"
#     -> black webview / "Failed to spawn child process" abort.
# An rpath lives in the ELF, so it survives env sanitizing and CWD moves.
# $ORIGIN/../.. from the helper dir is the flat usr/lib where linuxdeploy put
# the bundle libs.
if ! command -v patchelf >/dev/null 2>&1; then
    echo "patchelf not found — attempting install (needed to bake helper rpaths)..."
    (apt-get install -y patchelf 2>/dev/null || sudo apt-get install -y patchelf 2>/dev/null) >/dev/null || true
fi
if ! command -v patchelf >/dev/null 2>&1; then
    echo "ERROR: patchelf is required to bake the WebKit helper rpaths (apt-get install -y patchelf)" >&2
    exit 1
fi
HELPER_RPATH='$ORIGIN/../..'
for h in WebKitWebProcess WebKitNetworkProcess WebKitGPUProcess MiniBrowser; do
    if [ -f "$APPDIR/$WK_REL/$h" ]; then
        patchelf --set-rpath "$HELPER_RPATH" "$APPDIR/$WK_REL/$h"
        echo "  helper rpath '$HELPER_RPATH': $h"
    fi
done

# Phase 2c — complete the webkit closure inside the bundle.
# linuxdeploy's ldd-closure copy leaves out libraries it considers
# build-host-common (on the ubuntu 22.04 build host: libharfbuzz, libfribidi,
# libfreetype, libfontconfig, libexpat, libz). A minimal host WITHOUT
# webkit2gtk installed does not have those, so the bundled libwebkit could not
# load them. Walk the NEEDED closure of the webkit stack and copy whatever is
# missing from the build host, each with an $ORIGIN rpath. The glibc/libstdc++
# family stays host-provided (the floor is enforced by check-glibc-floor.sh);
# the graphics stack (X11/GL/GBM/Wayland) is NOT left to the host — Phase 2d
# bundles it (bundling only PART of the graphics stack is the failure mode
# that used to break WebKit EGL init on newer hosts with EGL_BAD_PARAMETER,
# so Phase 2d ships the whole family as one consistent set).
HOST_LIBDIR="$(dirname "$WK_SRC")"   # the build host's multiarch lib dir
host_provided() {
    case "$1" in
        libc.so*|libm.so*|libpthread.so*|libdl.so*|librt.so*|libresolv.so*|libutil.so*|\
        libstdc++.so*|libgcc_s.so*|ld-linux-*)
            return 0 ;;
        *) return 1 ;;
    esac
}
closure_queue=("$LIBWK")
JSC="$(find "$APPDIR/usr/lib" -maxdepth 1 -name 'libjavascriptcoregtk-4.1.so.0*' -type f 2>/dev/null | head -1)"
[ -n "$JSC" ] && closure_queue+=("$JSC")
declare -A seen_closure
while [ "${#closure_queue[@]}" -gt 0 ]; do
    next=()
    for lib in "${closure_queue[@]}"; do
        while IFS= read -r need; do
            [ -z "$need" ] && continue
            [ -n "${seen_closure[$need]:-}" ] && continue
            seen_closure[$need]=1
            if host_provided "$need"; then continue; fi
            if [ ! -e "$APPDIR/usr/lib/$need" ]; then
                hostlib="$HOST_LIBDIR/$need"
                if [ -e "$hostlib" ]; then
                    cp -L "$hostlib" "$APPDIR/usr/lib/$need"
                    patchelf --set-rpath '$ORIGIN' "$APPDIR/usr/lib/$need"
                    echo "  bundled webkit-closure lib: $need"
                    next+=("$APPDIR/usr/lib/$need")
                fi
            else
                # Already bundled: no copy needed, but walk its own NEEDEDs so
                # gaps behind bundled libs (e.g. libfribidi behind libpango)
                # are found too.
                next+=("$APPDIR/usr/lib/$need")
            fi
        done < <(readelf -d "$lib" 2>/dev/null | awk -F'[][]' '/NEEDED/ {print $2}')
    done
    closure_queue=("${next[@]}")
done

# The gdk-pixbuf SVG loader dlopen's librsvg at runtime (invisible to the
# NEEDED walk). Bundle it when the build host has it, matching what the gtk
# plugin did on the CI build host.
for rsvg in "$HOST_LIBDIR"/librsvg-2.so*; do
    [ -e "$rsvg" ] || continue
    base="$(basename "$rsvg")"
    [ -e "$APPDIR/usr/lib/$base" ] && continue
    cp -L "$rsvg" "$APPDIR/usr/lib/$base"
    patchelf --set-rpath '$ORIGIN' "$APPDIR/usr/lib/$base"
    echo "  bundled svg loader lib: $base"
done

# Phase 2d — bundle the graphics runtime closure (wayland + EGL + GL + glvnd +
# mesa DRI) so a STOCK host with no graphics stack at all can run the app.
# Field proof (real Ubuntu VMs): stock 22.04 exited 127 with
# "strikehub-real: error while loading shared libraries: libwayland-client.so.0"
# (ldd: libgbm.so.1 + libEGL.so.1 also missing; ks-connector missing
# libwayland-client.so.0); stock 26.04 failed the same way on libEGL.so.1;
# with the graphics stack installed the app launches to sign-in. CI never
# caught it because the ubuntu-22.04 runners ship the wayland/mesa stack and
# the verify step only ever asserted the gtk/webkit/xdo libs.
#
# What the bundle must carry, and why each piece:
#   - libwayland-client: NEEDED by the app binaries (winit/tao), ks-connector,
#     libgdk-3 and libwebkit2gtk themselves; libgbm: NEEDED by libwebkit2gtk
#     and libgstgl; libEGL/libGL: NEEDED by libgstgl. These satisfy the
#     dynamic linker on a stock host (the exit-127 class).
#   - libEGL.so.1/libgbm.so.1 are ALSO dlopen'd by WebKit at startup
#     (invisible to any NEEDED walk) — the web process aborts with
#     "Could not create default EGL display" without them.
#   - The glvnd dispatchers (libEGL/libGL/libGLX/libOpenGL + libGLdispatch)
#     dlopen the mesa vendor modules (libEGL_mesa/libGLX_mesa) located via
#     __GLX_DRIVERS_PATH (set by the AppRun hook); mesa then dlopens a DRI
#     driver from LIBGL_DRIVERS_PATH. Without a bundled swrast_dri.so
#     (llvmpipe, which NEEEds libLLVM) a GPU-less container/server image
#     cannot render the webview at all.
# The whole family ships as ONE consistent set from the build host (22.04-era
# on CI): bundling only part of it mixes versions across the EGL boundary and
# breaks init on hosts with a different mesa (the old EGL_BAD_PARAMETER bug).
# Every copy gets a $ORIGIN rpath and the binaries already carry RUNPATHs into
# usr/lib, so all lookups stay in-bundle on any host. Only the glibc family
# stays host-provided (check-glibc-floor.sh enforces the floor).
DRI_SRC="$(dirname "$(find -L "$HOST_LIBDIR" -maxdepth 2 -name swrast_dri.so 2>/dev/null | head -1)")"
for core in libwayland-client.so.0 libwayland-egl.so.1 libgbm.so.1 libEGL.so.1 \
            libGL.so.1 libGLX.so.0 libOpenGL.so.0 libGLdispatch.so.0 \
            libGLX_mesa.so.0 libEGL_mesa.so.0 libdrm.so.2 libglapi.so.0; do
    [ -e "$HOST_LIBDIR/$core" ] || {
        echo "ERROR: $core not found on the build host ($HOST_LIBDIR) — cannot bundle the graphics runtime (install libegl1 libgbm1 libwayland-client0 libgl1 libgl1-mesa-dri)" >&2
        exit 1
    }
done
[ -n "$DRI_SRC" ] && [ -f "$DRI_SRC/swrast_dri.so" ] || {
    echo "ERROR: no mesa DRI drivers (swrast_dri.so) under $HOST_LIBDIR — install libgl1-mesa-dri on the build host" >&2
    exit 1
}

# Copy the core family members themselves. The NEEDED walk below only copies
# what is NEEDED by something; the glvnd vendor modules (libEGL_mesa/
# libGLX_mesa) and libOpenGL are dlopen'd at runtime — never NEEDED — so they
# would be silently missed and dlopen would fall through to the (stock host's
# empty) system paths: "Could not create default EGL display".
for core in libwayland-client.so.0 libwayland-egl.so.1 libgbm.so.1 libEGL.so.1 \
            libGL.so.1 libGLX.so.0 libOpenGL.so.0 libGLdispatch.so.0 \
            libGLX_mesa.so.0 libEGL_mesa.so.0 libdrm.so.2 libglapi.so.0; do
    if [ ! -e "$APPDIR/usr/lib/$core" ]; then
        cp -L "$HOST_LIBDIR/$core" "$APPDIR/usr/lib/$core"
        patchelf --set-rpath '$ORIGIN' "$APPDIR/usr/lib/$core"
        echo "  bundled graphics core lib: $core"
    fi
done

echo "Bundling graphics runtime closure from $HOST_LIBDIR ..."
# Walk the NEEDED closure of the graphics family (same walk as Phase 2c, but
# the ONLY exclusion is the glibc family: X11/xcb/LLVM companions of the new
# libs are bundled too, so nothing graphics-related resolves to the host).
graphics_host_provided() {
    case "$1" in
        libc.so*|libm.so*|libpthread.so*|libdl.so*|librt.so*|libresolv.so*|libutil.so*|\
        libstdc++.so*|libgcc_s.so*|ld-linux-*)
            return 0 ;;
        *) return 1 ;;
    esac
}
g_queue=("$HOST_LIBDIR/libwayland-client.so.0" "$HOST_LIBDIR/libwayland-egl.so.1" \
         "$HOST_LIBDIR/libgbm.so.1" "$HOST_LIBDIR/libEGL.so.1" "$HOST_LIBDIR/libGL.so.1" \
         "$HOST_LIBDIR/libGLX.so.0" "$HOST_LIBDIR/libOpenGL.so.0" "$HOST_LIBDIR/libGLdispatch.so.0" \
         "$HOST_LIBDIR/libGLX_mesa.so.0" "$HOST_LIBDIR/libEGL_mesa.so.0" \
         "$HOST_LIBDIR/libdrm.so.2" "$HOST_LIBDIR/libglapi.so.0" \
         "$DRI_SRC/swrast_dri.so")
declare -A seen_graphics
while [ "${#g_queue[@]}" -gt 0 ]; do
    g_next=()
    for lib in "${g_queue[@]}"; do
        while IFS= read -r need; do
            [ -z "$need" ] && continue
            [ -n "${seen_graphics[$need]:-}" ] && continue
            seen_graphics[$need]=1
            graphics_host_provided "$need" && continue
            if [ -e "$APPDIR/usr/lib/$need" ]; then
                # Already bundled (linuxdeploy or Phase 2c) — walk it so gaps
                # behind it are found, but keep the existing copy.
                g_next+=("$APPDIR/usr/lib/$need")
            elif [ -e "$HOST_LIBDIR/$need" ]; then
                cp -L "$HOST_LIBDIR/$need" "$APPDIR/usr/lib/$need"
                patchelf --set-rpath '$ORIGIN' "$APPDIR/usr/lib/$need"
                echo "  bundled graphics lib: $need"
                g_next+=("$APPDIR/usr/lib/$need")
            fi
        done < <(readelf -d "$lib" 2>/dev/null | awk -F'[][]' '/NEEDED/ {print $2}')
    done
    g_queue=("${g_next[@]}")
done

# Mesa DRI driver dir: on 22.04 every driver name is a hardlink of one ~32MB
# gallium blob (SONAME libgallium_dri.so, llvmpipe included). Preserve the
# hardlinks so the AppDir holds a single copy (mksquashfs dedupes anyway).
DRI_REL="${DRI_SRC#/}"
mkdir -p "$APPDIR/$DRI_REL"
copied_dri=0
for d in "$DRI_SRC"/*_dri.so; do
    [ -e "$d" ] || continue
    base="$(basename "$d")"
    [ -e "$APPDIR/$DRI_REL/$base" ] && continue
    cp -al "$d" "$APPDIR/$DRI_REL/$base" 2>/dev/null || cp -L "$d" "$APPDIR/$DRI_REL/$base"
    copied_dri=$((copied_dri + 1))
done
[ "$copied_dri" -gt 0 ] || { echo "ERROR: no DRI driver files copied from $DRI_SRC" >&2; exit 1; }
echo "Bundled DRI drivers: $copied_dri driver names -> $APPDIR/$DRI_REL (incl. swrast software renderer)"

# Phase 3 — package the fully-populated AppDir (linuxdeploy already wrote AppRun).
# --runtime-file pins the embedded runtime to the checked static FUSE-3
# type2-runtime (project-management#382): without it, appimagetool would
# download its own (moving) default runtime and the FUSE guarantee would
# silently drift.
echo "Packaging AppImage (static FUSE-3 type2-runtime ${TYPE2_RUNTIME_VERSION})..."
ARCH=$ARCH "./${APPIMAGETOOL}" --runtime-file "$PWD/${TYPE2_RUNTIME}" "$APPDIR" "StrikeHub-${VERSION}-${ARCH}.AppImage"

echo ""
echo "✅ Portable AppImage created: StrikeHub-${VERSION}-${ARCH}.AppImage"
echo "   (GTK/WebKit runtime + graphics stack bundled — runs out of the box on stock Ubuntu, no host webkit2gtk/mesa needed)"
echo "   (static FUSE-3 type2-runtime ${TYPE2_RUNTIME_VERSION} embedded — no host libfuse2 needed; stock Ubuntu 24.04+ OK)"
