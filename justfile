# Contributor and maintainer commands. Each recipe calls one script.

set positional-arguments

# List the recipes
[private]
default:
    @{{ just_executable() }} --list --unsorted

# Build and run the tests (GitHub CI gate)
check:
    bash scripts/check.sh

# Also run fmt, Clippy, docs and release-tool tests (Forgejo CI gate)
check-full:
    bash scripts/check.sh --full

# Check that the version files agree
release-validate:
    python3 scripts/release.py validate

# Bump the version: patch, minor or major; add --dry-run to preview
release-prepare *args:
    python3 scripts/release.py prepare "$@"

# Check a release tag name, for example fjx/v0.3.1
release-validate-tag tag:
    python3 scripts/release.py validate-tag "$1"

# Build release archives: DIR [TARGET|all]
package *args:
    python3 scripts/package_release.py "$@"

# Verify release archives: DIR VERSION --source-date-epoch EPOCH
package-verify *args:
    python3 scripts/package_release.py verify "$@"
