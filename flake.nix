{
  description = "Leeroy - a TUI for Jenkins";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        # vhs 0.12.0 exits 0 without writing any output (canceled context
        # prevents ffmpeg from starting): charmbracelet/vhs#787. Pin 0.11.0
        # until a fixed release lands in nixpkgs.
        vhs = pkgs.vhs.overrideAttrs (old: rec {
          version = "0.11.0";
          src = pkgs.fetchFromGitHub {
            owner = "charmbracelet";
            repo = "vhs";
            rev = "v${version}";
            hash = "sha256-VOiI+ddiax04QtCcDr6ze53kd/HHGbfQE3j/32iq4Ro=";
          };
          vendorHash = "sha256-cgKLYUATtn4hMdIOXZe9JWYNUOrX3S6BDfvS+rIWDfM=";
        });
      in
      {
        devShells.default = pkgs.mkShell {
          packages = [ vhs ] ++ (with pkgs; [
            rustc
            cargo
            clippy
            rustfmt
            rust-analyzer
            cargo-insta
            tmux
            ttyd
            ffmpeg
            python3
            openssl
          ]);
          RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
        };
      });
}
