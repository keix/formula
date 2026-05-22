{
  description = "formula - Z80 CPU emulator written in Rust";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { nixpkgs, flake-utils, ... }: flake-utils.lib.eachSystem [
    "x86_64-linux"
    "aarch64-linux"
    "aarch64-darwin"
  ] (system:
    let
      pkgs = import nixpkgs { inherit system; };
      # minifb dlopens these at runtime on Linux for the X11 / Wayland
      # backends; on Darwin it uses Cocoa via the SDK and needs nothing.
      linuxDisplayLibs = with pkgs; [
        libxkbcommon
        wayland
        libx11
        libxcursor
        libxi
        libxrandr
      ];
    in
    {
      devShells.default = pkgs.mkShell {
        packages = with pkgs; [
          cargo
          rustc
          rustfmt
          clippy
          rust-analyzer
        ];
        buildInputs = pkgs.lib.optionals pkgs.stdenv.isLinux linuxDisplayLibs;
        LD_LIBRARY_PATH = pkgs.lib.optionalString pkgs.stdenv.isLinux
          (pkgs.lib.makeLibraryPath linuxDisplayLibs);
      };
    }
  );
}
