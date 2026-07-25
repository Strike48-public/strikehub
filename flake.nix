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
      desktopLibs = with pkgs; [
        gtk3 webkitgtk_4_1 libsoup_3 glib gdk-pixbuf cairo pango atk
        openssl
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
