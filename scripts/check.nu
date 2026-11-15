#!/usr/bin/env nu

# The merge gate builds the product and runs its local CLI fixtures.
# --full adds maintenance and release-tool checks. Neither mode publishes.
const project_dir = path self ..

def run-check [program: string, arguments: list<string>] {
  print $"> ($program) ($arguments | str join ' ')"
  ^$program ...$arguments
  if $env.LAST_EXIT_CODE != 0 {
    error make {msg: $"check failed: ($program) ($arguments | str join ' ')"}
  }
}

def main [--full] {
  if (version | get version) != '0.112.2' {
    error make {msg: 'fjx checks require Nushell 0.112.2'}
  }
  cd $project_dir
  # Fixture overrides must not redirect a contributor's final version check.
  hide-env --ignore-errors RELEASE_WORKSPACE_ROOT
  # Checkout uses the job token; build scripts and tests do not need it.
  hide-env --ignore-errors FJX_TOKEN FORGEJO_TOKEN GITEA_TOKEN GITHUB_TOKEN ACTIONS_RUNTIME_TOKEN
  let prerequisites = if $full {
    [cargo git bash script tar gzip zip unzip file sha256sum mktemp]
  } else { [cargo git bash script] }
  for program in $prerequisites {
    if (which --all $program | where type == external | is-empty) {
      error make {msg: $"missing test prerequisite: ($program); see README.md"}
    }
  }

  run-check cargo [build --locked]
  run-check cargo [test --locked --all-targets]
  if not $full {
    print 'Build and E2E checks passed.'
    return
  }

  run-check cargo [fmt --all -- --check]
  run-check cargo [clippy --locked --all-targets -- -D warnings]
  with-env {RUSTDOCFLAGS: '-D warnings'} {
    run-check cargo [doc --locked --no-deps]
  }
  run-check $nu.current-exe [--no-config-file tests/repository.nu]
  run-check $nu.current-exe [--no-config-file tests/release.nu]
  run-check $nu.current-exe [--no-config-file tests/forgejo-ci.nu]
  run-check $nu.current-exe [--no-config-file scripts/release.nu validate]
  print 'All fjx checks passed.'
}
