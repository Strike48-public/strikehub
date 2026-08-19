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
          # buildInputs only affects compile/link env, not the runtime loader,
          # so desktop/test binaries that dlopen GTK/WebKit/openssl need these
          # on LD_LIBRARY_PATH to run under `nix develop`.
          export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath desktopLibs}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"

          # GIO loads its TLS backend (from glib-networking) as a dynamic module
          # discovered via GIO_EXTRA_MODULES. Point it at the gio-modules dir so
          # WebKit/libsoup can do HTTPS (otherwise: "TLS support not available").
          export GIO_EXTRA_MODULES="${pkgs.glib-networking}/lib/gio/modules''${GIO_EXTRA_MODULES:+:$GIO_EXTRA_MODULES}"
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
