{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };
  outputs = {
    self,
    nixpkgs,
    rust-overlay,
    flake-utils,
  }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        overlays = [(import rust-overlay)];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [
            "rust-src"
            "rust-analyzer"
            "clippy"
            "rustfmt"
          ];
          # docs/development.md prerequisites: the web/wasm target (cargo xtask wasm).
          targets = [ "wasm32-unknown-unknown" ];
        };
        nativeBuildInputs = with pkgs; [
          pkg-config
        ];
        runtimeLibs = with pkgs; [
          # docs/development.md › Linux prereqs:
          # libxkbcommon-dev, libwayland-dev, libx11-dev, libxrandr-dev,
          # libxi-dev, libgl1-mesa-dev, libgtk-3-dev (+ fontconfig).
          libxkbcommon
          wayland
          libx11
          libxrandr
          libxi
          libGL
          gtk3
          fontconfig

          # winit 0.30 (eframe)
          libxcursor
          libxfixes
          libxcb

          vulkan-loader
          freetype
          glib
          dbus
          at-spi2-core
          udev
        ];
      in {
        devShells.default = pkgs.mkShell {
          nativeBuildInputs = nativeBuildInputs;
          # Faster test loops: `cargo nextest run` (docs/development.md).
          buildInputs = runtimeLibs ++ [rustToolchain pkgs.cargo-nextest];
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath runtimeLibs;
          shellHook = ''
            export XDG_DATA_DIRS="${pkgs.gtk3}/share:${pkgs.gsettings-desktop-schemas}/share:''${XDG_DATA_DIRS:-/usr/share}"
          '';
        };
      }
    );
}
