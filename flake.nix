{
  description = "Stormlight — clean MOBA engine (Bevy 0.18 + Lightyear, Wayland/Hyprland), one dev shell for engine + mods.";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      flake-utils,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };

        # Pinned nightly + the components the workspace's rust-toolchain.toml
        # asks for, plus the wasm + windows cross targets.
        rustNightly = pkgs.rust-bin.nightly.latest.default.override {
          extensions = [
            "rust-src"
            "llvm-tools-preview"
            "clippy"
            "rustfmt"
          ];
          targets = [
            "x86_64-unknown-linux-gnu"
            "x86_64-pc-windows-gnu"
            "wasm32-unknown-unknown"
          ];
        };

        cargo-codspeed-fixed = pkgs.cargo-codspeed.overrideAttrs (old: {
          doCheck = false; # tests fail the build on current nightly
        });

        # Runtime libs Bevy needs (Wayland-first for Hyprland, X11 fallback).
        bevyDeps = with pkgs; [
          wayland
          wayland-protocols
          libxkbcommon

          xorg.libX11
          xorg.libXcursor
          xorg.libXrandr
          xorg.libXi

          vulkan-loader
          vulkan-headers
          vulkan-validation-layers
          libGL

          alsa-lib

          pkg-config
          openssl.dev
          udev
        ];

        # Code-diagnostics / profiling / test toolbox.
        rustTools = with pkgs; [
          cargo-codspeed-fixed
          cargo-nextest      # the ONLY blessed test runner
          cargo-bolero       # property + fuzz campaigns
          cargo-fuzz
          cargo-mutants
          cargo-tarpaulin
          cargo-llvm-cov
          cargo-flamegraph
          cargo-show-asm
          cargo-llvm-lines
          cargo-bloat
          cargo-expand
          cargo-pgo
          cargo-geiger
          cargo-spellcheck
          cargo-deny
          cargo-audit
          cargo-machete
          bacon
        ];

        # Native toolchain (linker, debuggers) + Windows cross + wasm tools.
        nativeTools = with pkgs; [
          mold
          clang
          llvmPackages.bintools
          llvmPackages.llvm
          gdb
          valgrind
          heaptrack
          # Cross-compile to native Windows (mirrors the GitLab CI job).
          pkgsCross.mingwW64.stdenv.cc
          # WASM mod tooling.
          wabt
          binaryen
          wasm-tools
        ];

      in
      {
        devShells.default = pkgs.mkShell {
          buildInputs = [ rustNightly ] ++ rustTools ++ nativeTools ++ bevyDeps;

          # vulkan / wayland / libGL must resolve at runtime.
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (
            with pkgs;
            [
              wayland
              vulkan-loader
              libGL
              libxkbcommon
            ]
          );

          shellHook = ''
            echo "🌩  Stormlight dev shell — Rust $(rustc --version | cut -d' ' -f2) + mold (LLVM backend; fuzz/cov friendly)"
            echo "    engine: cargo t   ·   lint: cargo lint   ·   mods: (cd mods && cargo build --target wasm32-unknown-unknown --release)"
          '';
        };
      }
    );
}
