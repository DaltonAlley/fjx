#!/usr/bin/env nu

# One check entry point for contributors and CI. Never publishes or changes versions.
const project_dir = path self ..

def run-check [program: string, arguments: list<string>] {
  print $"> ($program) ($arguments | str join ' ')"
  ^$program ...$arguments
  if $env.LAST_EXIT_CODE != 0 {
    error make {msg: $"check failed: ($program) ($arguments | str join ' ')"}
  }
}

def main [] {
  if (version | get version) != '0.112.2' {
    error make {msg: 'fjx checks require Nushell 0.112.2'}
  }
  cd $project_dir
  # Fixture overrides must not redirect a contributor's final version check.
  hide-env --ignore-errors RELEASE_WORKSPACE_ROOT
  for program in [cargo git bash script tar gzip zip unzip sha256sum mktemp] {
    if (which --all $program | where type == external | is-empty) {
      error make {msg: $"missing test prerequisite: ($program); see README.md"}
    }
  }

  run-check cargo [fmt --all -- --check]
  run-check cargo [clippy --locked --all-targets -- -D warnings]
  run-check cargo [test --locked --all-targets]
  with-env {RUSTDOCFLAGS: '-D warnings'} {
    run-check cargo [doc --locked --no-deps]
  }
  run-check $nu.current-exe [--no-config-file tests/repository.nu]
  run-check $nu.current-exe [--no-config-file tests/release.nu]
  run-check $nu.current-exe [--no-config-file scripts/release.nu validate]
  print 'All fjx checks passed.'
}
