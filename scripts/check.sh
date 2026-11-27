#!/usr/bin/env bash
# The merge gate builds the product and runs its local CLI fixtures.
# --full adds maintenance and release-tool checks. Neither mode publishes.
set -euo pipefail

nu_version=0.112.2

fail() {
  printf 'error: %s\n' "$1" >&2
  exit 1
}

run_check() {
  printf '> %s\n' "$*"
  if ! "$@"; then
    fail "check failed: $*"
  fi
}

full=false
for argument in "$@"; do
  case "$argument" in
    --full) full=true ;;
    -h | --help)
      printf 'usage: scripts/check.sh [--full]\n'
      exit 0
      ;;
    *) fail "unknown argument: $argument; usage: scripts/check.sh [--full]" ;;
  esac
done

script_dir="$(dirname "${BASH_SOURCE[0]}")"
cd "$script_dir/.."
# Fixture overrides must not redirect a contributor's final version check.
unset RELEASE_WORKSPACE_ROOT
# Checkout uses the job token; build scripts and tests do not need it.
unset FJX_TOKEN FORGEJO_TOKEN GITEA_TOKEN GITHUB_TOKEN ACTIONS_RUNTIME_TOKEN

prerequisites=(cargo git bash script)
if $full; then
  # Release tools are Python; only tests/repository.nu still needs Nushell.
  prerequisites+=(python3 nu tar gzip zip unzip file sha256sum mktemp)
fi
for program in "${prerequisites[@]}"; do
  if ! type -P "$program" >/dev/null; then
    fail "missing test prerequisite: $program; see README.md"
  fi
done
if $full && [[ "$(nu --version)" != "$nu_version" ]]; then
  fail "fjx checks require Nushell $nu_version"
fi
if $full && ! python3 -c 'import sys; sys.exit(sys.version_info < (3, 11))'; then
  fail "fjx checks require Python 3.11 or newer"
fi

run_check cargo build --locked
run_check cargo test --locked --all-targets
if ! $full; then
  printf 'Build and E2E checks passed.\n'
  exit 0
fi

run_check cargo fmt --all -- --check
run_check cargo clippy --locked --all-targets -- -D warnings
RUSTDOCFLAGS='-D warnings' run_check cargo doc --locked --no-deps
run_check nu --no-config-file tests/repository.nu
run_check python3 -m unittest discover --start-directory tests --pattern 'test_*.py'
run_check python3 scripts/release.py validate
printf 'All fjx checks passed.\n'
