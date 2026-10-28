{ pkgs ? import <nixpkgs> { } }:

# Build this package with the deployment host's nixpkgs. Credentials and runner
# registration files are created at runtime and never enter the Nix store.
pkgs.writeShellApplication {
  name = "fjx-install-runner";
  runtimeInputs = [ pkgs.coreutils pkgs.systemd ];
  text = ''
    exec ${pkgs.nushell}/bin/nu --no-config-file ${./install-runner.nu} \
      ${pkgs.forgejo-runner}/bin/forgejo-runner "$@"
  '';
}
