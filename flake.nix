{
  description = "StrikeHub dev environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { nixpkgs, fenix, ... }:
    let
      # Rust toolchain (stable >= 1.91.1, edition 2024). nixpkgs rustc may lag,
      # so pull a full stable toolchain from fenix.
      mkRust = system: (fenix.packages.${system}).combine [
        fenix.packages.${system}.stable.rustc
        fenix.packages.${system}.stable.cargo
        fenix.packages.${system}.stable.clippy
        fenix.packages.${system}.stable.rustfmt
        fenix.packages.${system}.stable.rust-analyzer
        fenix.packages.${system}.stable.rust-src
      ];

      # ----- Linux desktop + server dev shell -----
      linuxSystem = "x86_64-linux";
      pkgs = import nixpkgs { system = linuxSystem; };

      # Native libs for the wry/tao WebView (desktop feature) on Linux.
      # xdotool provides libxdo, which tao links for X11 input handling.
      desktopLibs = with pkgs; [
        gtk3 webkitgtk_4_1 libsoup_3 glib gdk-pixbuf cairo pango atk
        openssl xdotool
        # glib-networking supplies GIO's TLS backend (gnutls). WebKitGTK/libsoup
        # route HTTPS through it; without it *and* GIO_EXTRA_MODULES pointing at
        # its module dir, every https request in the webview fails with
        # "TLS support not available" (e.g. restty's CDN font fetches → the
        # terminal grid can't size and renders blank). See shellHook below.
        glib-networking
        # xz (liblzma.so.5) and bzip2 (libbz2.so.1) are dlopened transitively
        # at runtime by the WebKit/GTK stack; without them the desktop binary
        # (and test binaries linking it) fail to load their shared libraries.
        xz bzip2
        # libxcb (and X libs) are needed by connector child processes that
        # StrikeHub launches — e.g. the pick `pentest-agent`, which links
        # libxcb.so.1. Connectors inherit this shell's LD_LIBRARY_PATH, so
        # these must be present here for a sibling-workspace connector to run.
        xorg.libxcb libpcap dbus
        # EGL/GL provider. WebKitGTK dlopens libEGL.so.1, and binaries built
        # here use the Nix loader, which does not read /etc/ld.so.cache, so the
        # host's Mesa is invisible and nothing in WebKit's own RUNPATH supplies
        # EGL. Without these the web process aborts with "Could not create
        # default EGL display: EGL_BAD_PARAMETER" and the window never appears.
        libglvnd mesa libdrm
      ];

      # ----- macOS dev shell (toolchain + build tools only) -----
      darwinSystem = "aarch64-darwin";
      darwinPkgs = import nixpkgs { system = darwinSystem; };
    in {
      devShells.${linuxSystem}.default = pkgs.mkShell {
        packages = [ (mkRust linuxSystem) pkgs.just ];

        # protoc: the gRPC/tonic transport build. pkg-config: crate lib discovery.
        nativeBuildInputs = with pkgs; [ pkg-config protobuf ];
        buildInputs = desktopLibs;

        env = {
          # rusqlite / cc-rs host builds need a C compiler.
          CC = "cc";
        };

        shellHook = ''
          # Runtime libs (and the GIO TLS module dir) for the Nix-built desktop
          # and desktop-test binaries, which dlopen GTK/WebKit/openssl at runtime
          # (buildInputs only affects compile/link, not the loader).
          #
          # We deliberately DO NOT export these as LD_LIBRARY_PATH /
          # GIO_EXTRA_MODULES. direnv injects this shell's env into EVERY command
          # run in the repo, host binaries included, and a global LD_LIBRARY_PATH
          # force-loads Nix's glibc-2.42 libs (openssl, xz, ...) into system
          # tools built against glibc 2.39 -> "GLIBC_ABI_DT_X86_64_PLT not found"
          # (breaks ssh, scp, curl, and so `git push` and `just remote-build`).
          #
          # Instead export them under neutral names the loader ignores; the just
          # recipes that actually execute the desktop binary (`run`, `test`)
          # promote them to the real vars for that one process. Connectors the
          # app launches inherit LD_LIBRARY_PATH from the app's env, so they
          # still resolve their libs.
          export STRIKEHUB_RUNTIME_LIBS="${pkgs.lib.makeLibraryPath desktopLibs}"
          export STRIKEHUB_GIO_MODULES="${pkgs.glib-networking}/lib/gio/modules"

          # libglvnd dispatches to a vendor EGL named by these JSON manifests;
          # GBM_BACKENDS_PATH and LIBGL_DRIVERS_PATH replace Mesa's NixOS-only
          # /run/opengl-driver default, which does not exist on a non-NixOS host
          # (otherwise: "MESA-LOADER: failed to open dri" and software fallback).
          #
          # Setting __EGL_VENDOR_LIBRARY_DIRS REPLACES libglvnd's default search,
          # so the host dirs are listed after Mesa rather than dropped: a machine
          # whose GPU needs a non-Mesa vendor ICD (the proprietary NVIDIA driver
          # ships 10_nvidia.json there) would otherwise be left with a Mesa EGL
          # that cannot drive it. Mesa stays first, so Mesa-backed GPUs (Intel,
          # AMD via radeonsi, nouveau) keep the path verified here.
          export __EGL_VENDOR_LIBRARY_DIRS="${pkgs.mesa}/share/glvnd/egl_vendor.d:/usr/share/glvnd/egl_vendor.d:/etc/glvnd/egl_vendor.d"
          export GBM_BACKENDS_PATH="${pkgs.mesa}/lib/gbm"
          export LIBGL_DRIVERS_PATH="${pkgs.mesa}/lib/dri"
        '';
      };

      # macOS: Rust toolchain + protoc + pkg-config. The desktop WebView stack
      # comes from the system WebKit on macOS, so no GTK libs here.
      devShells.${darwinSystem}.default = darwinPkgs.mkShell {
        packages = [ (mkRust darwinSystem) darwinPkgs.just ];
        nativeBuildInputs = with darwinPkgs; [ pkg-config protobuf ];
      };
    };
}
