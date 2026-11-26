#!/usr/bin/env nu

use release.nu [validate-versions version-greater publication-state create-release]
use package-release.nu [release-targets verify-assets]

const project_dir = path self ..
const release_script = path self release.nu
const package_script = path self package-release.nu

def fail [message: string] { error make {msg: $"fjx Forgejo CI: ($message)"} }

def checked [program: string, arguments: list<string>] {
  ^$program ...$arguments
  if $env.LAST_EXIT_CODE != 0 { fail $"($program) failed; publication stopped" }
}

# A release requires an actual increase across this push, rather than a missing
# tag. Ordinary commits and initial imports must never publish an old version.
export def release-candidate [before: string, revision: string] {
  let version = validate-versions
  if $revision !~ '^[0-9a-f]{40}([0-9a-f]{24})?$' or (^git rev-parse HEAD | str trim) != $revision {
    fail 'checked-out revision does not match the push'
  }
  if $before =~ '^0{40}(0{24})?$' { return false }
  if $before !~ '^[0-9a-f]{40}([0-9a-f]{24})?$' { fail 'push has no valid before revision' }
  let ancestor = do { ^git merge-base --is-ancestor $before $revision } | complete
  if $ancestor.exit_code != 0 { fail 'before revision must be an ancestor of the checked commit' }
  let previous = do { ^git show $"($before):release.toml" } | complete
  if $previous.exit_code != 0 { fail 'could not read release.toml before this push' }
  let old_version = try { ($previous.stdout | from toml).version } catch { fail 'invalid previous release.toml' }
  if ($old_version | describe) != string or $old_version !~ '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$' {
    fail 'invalid previous version'
  }
  if $old_version == $version { return false }
  if not (version-greater $version $old_version) { fail 'release version must increase' }
  true
}

# Runs inside the pinned cargo-zigbuild image; no container socket is needed.
export def build-assets [revision: string] {
  cd $project_dir
  hide-env --ignore-errors RELEASE_WORKSPACE_ROOT
  # Only the publication helpers need authentication, never Cargo build scripts.
  hide-env --ignore-errors FJX_TOKEN FORGEJO_TOKEN GITEA_TOKEN GITHUB_TOKEN ACTIONS_RUNTIME_TOKEN
  let version = validate-versions
  if (^git rev-parse HEAD | str trim) != $revision { fail 'release checkout changed' }
  let epoch = ^git show -s --format=%ct $revision | str trim | into int
  if $epoch <= 0 { fail 'release commit has no valid source timestamp' }
  if ('publish-assets' | path exists) { fail 'publish-assets must not already exist' }
  let binaries = mktemp -d
  try {
    for target in (release-targets) {
      checked cargo [zigbuild --locked --release --target $target.triple]
      let directory = $binaries | path join $target.triple
      mkdir $directory
      let binary = $env.CARGO_TARGET_DIR? | default ($project_dir | path join target) | path join $target.triple release $target.executable
      cp $binary ($directory | path join $target.executable)
      if $target.runtime == linux/amd64 {
        let result = do { ^$binary --version } | complete
        if $result.exit_code != 0 or ($result.stdout | str trim) != $"fjx ($version)" {
          fail $"Linux runtime smoke failed for ($target.triple)"
        }
      }
    }
    with-env {FJX_PACKAGE_BIN_ROOT: $binaries} {
      checked $nu.current-exe [--no-config-file $package_script publish-assets all --source-date-epoch ($epoch | into string)]
    }
    verify-assets publish-assets $version
  } catch {|error|
    rm --recursive --force $binaries
    error make $error.raw
  }
  rm --recursive --force $binaries
}

def main-push [] {
  ($env.FORGEJO_REF? | default '') == 'refs/heads/main' and ($env.FORGEJO_EVENT_NAME? | default '') == push
}

def "main candidate" [before: string, revision: string, output: path] {
  cd $project_dir
  hide-env --ignore-errors RELEASE_WORKSPACE_ROOT
  let candidate = if (main-push) { release-candidate $before $revision } else { false }
  $"release=($candidate)\n" | save --append --raw $output
}

def "main build" [revision: string] { build-assets $revision }

def "main publish" [revision: string] {
  cd $project_dir
  hide-env --ignore-errors RELEASE_WORKSPACE_ROOT
  if not (main-push) {
    fail 'publication is only allowed for a push to main'
  }
  if not (release-candidate ($env.BEFORE_SHA? | default '') $revision) {
    fail 'publication requires a version increase across this push'
  }
  let epoch = ^git show -s --format=%ct $revision | str trim
  # Reconcile remote state before building, and again immediately before writes.
  publication-state $revision $epoch | ignore
  build-assets $revision
  let plan = publication-state $revision $epoch
  # Create the draft and its assets before the tag exists. Forgejo publishes a
  # draft as soon as it syncs a pushed tag with the same name, so publish-release
  # pushes the tag only after every remote byte matches the local build.
  let release_id = if $plan.needs_release {
    create-release $plan.version $plan.tag | into string
  } else { $plan.release_id }
  if ($plan.missing_assets | is-not-empty) {
    checked $nu.current-exe [--no-config-file $release_script upload-missing-assets $release_id $plan.missing_assets]
  }
  if $plan.needs_publication {
    let local_tag = do { ^git rev-parse --quiet --verify $"refs/tags/($plan.tag)" } | complete
    if $plan.needs_tag and $local_tag.exit_code != 0 {
      checked git [-c 'user.name=Forgejo Actions' -c 'user.email=actions@localhost' tag --annotate $plan.tag --message $"fjx v($plan.version)" $revision]
    }
    checked $nu.current-exe [--no-config-file $release_script publish-release $plan.version $revision $plan.tag]
  }
  checked $nu.current-exe [--no-config-file $release_script verify-final $plan.version $revision $plan.tag]
  print $"Published and verified ($plan.tag)."
}

def main [] { fail 'usage: scripts/forgejo-ci.nu candidate BEFORE SHA OUTPUT | build SHA | publish SHA' }
