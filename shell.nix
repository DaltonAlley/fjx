{ pkgs ? import <nixpkgs> { } }:

let
  rustOverlay = import (builtins.fetchTarball {
    url = "https://github.com/oxalica/rust-overlay/archive/fad80eb74614f7dc37c1c83df8caae64361eda22.tar.gz";
    sha256 = "1nb95ws7z9z803ilhd0x08clqfpljpfa8lf17bz8mza73hd1jfn2";
  });
  rustPkgs = pkgs.extend rustOverlay;
  toolchain = rustPkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
in
assert pkgs.nushell.version == "0.112.2";
pkgs.mkShell {
  packages = [
    toolchain
    pkgs.nushell
    pkgs.git
    pkgs.bash
    pkgs.util-linux
    pkgs.coreutils
    pkgs.gnutar
    pkgs.gzip
    pkgs.zip
    pkgs.unzip
    pkgs.file
  ];
}
