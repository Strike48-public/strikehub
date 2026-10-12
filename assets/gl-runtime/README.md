# Vendored GL runtime (software renderer)

`gl-runtime-<arch>.tar.xz` is the GL/EGL stack the AppImage ships instead of
the build host's mesa. It exists because:

- WebKitGTK >= 2.42 **requires** an EGL display in the WebProcess (it aborts
  with `Could not create default EGL display: EGL_BAD_PARAMETER` without one).
- Ubuntu 22.04-era mesa (23.2) **cannot** initialize an EGL display on a
  GPU-less host (Xvfb / a container / a VM whose X server has no DRI2/DRI3):
  its X11 swrast path needs the DRI2 wire protocol, which Xvfb does not
  implement. Verified on stock 22.04 and 26.04 containers and VMs.
- Every newer-mesa distro binary (Ubuntu 24.04/26.04, Debian) is linked
  against a glibc above the bundle's floor (GLIBC_2.35), so those packages
  cannot simply be copied in.

So the runtime is built **from source on the oldest supported base**
(Ubuntu 22.04, glibc 2.35), deliberately minimal:

- **mesa 26.0.8**, classic (non-glvnd) build:
  `-Dplatforms=x11 -Degl=enabled -Dglx=dri -Dgbm=enabled -Dglvnd=disabled
  -Dopengl=true -Dgles1=disabled -Dgles2=disabled
  -Dgallium-drivers=softpipe -Dvulkan-drivers=[] -Dvideo-codecs=[]
  -Dllvm=disabled`
- **libdrm 2.4.124** (same toolchain).
- Only `libEGL.so.1`, `libGL.so.1`, `libgallium-26.0.8.so`, `libgbm.so.1`,
  `libdrm.so.2` and `usr/lib/x86_64-linux-gnu/dri/swrast_dri.so` ship.
- All with `$ORIGIN` rpaths (DRI driver: `$ORIGIN/../../..`) so dependency
  resolution never leaks to the host. `strip --strip-unneeded`.

## Rebuild recipe (verification / rotation)

Run in an `ubuntu:22.04` container:

```sh
apt-get update && apt-get install -y build-essential pkg-config meson ninja-build \
  python3-pip bison flex file patchelf xz-utils \
  libx11-dev libxcb1-dev libx11-xcb-dev libxcb-dri3-dev libxcb-present-dev \
  libxcb-shm0-dev libxcb-xfixes0-dev libxcb-randr0-dev libxcb-glx0-dev \
  libxshmfence-dev libxext-dev libxxf86vm-dev libexpat1-dev zlib1g-dev \
  libwayland-dev
python3 -m pip install 'meson>=1.4'

# libdrm 2.4.124 -> $OUT, then:
curl -sL https://archive.mesa3d.org/mesa-26.0.8.tar.xz | tar xJ
cd mesa-26.0.8
PKG_CONFIG_PATH="$OUT/lib/x86_64-linux-gnu/pkgconfig" \
meson setup build --prefix="$OUT" -Dbuildtype=release \
  -Dplatforms=x11 -Degl=enabled -Dglx=dri -Dgbm=enabled -Dglvnd=disabled \
  -Dopengl=true -Dgles1=disabled -Dgles2=disabled \
  -Dgallium-drivers=softpipe -Dvulkan-drivers=[] -Dvideo-codecs=[] \
  -Dllvm=disabled -Ddri-drivers-path="$OUT/dri"
ninja -C build install
```

Then package the files listed above (AppDir layout: `usr/lib/...`,
`usr/lib/x86_64-linux-gnu/dri/...`), `strip --strip-unneeded`,
`patchelf --set-rpath '$ORIGIN'` (`'$ORIGIN/../../..'` for the DRI driver),
`tar cJf`, and update the `.sha256`.

## Trade-off

The renderer is CPU-only (softpipe): hosts with a GPU also render in
software. That matches the previous behaviour in practice (the old bundle
shipped the 22.04 mesa whose DRI drivers needed a matching host userspace,
and fell back to llvmpipe everywhere except a few Intel setups), but it is
now explicit and — unlike before — it actually renders on GPU-less hosts.

Verified under Xvfb (`-screen 0 1280x800x24`) on: stock Ubuntu 22.04,
stock Ubuntu 26.04, and NixOS — WebKitWebProcess stays alive and the sign-in
UI paints.
