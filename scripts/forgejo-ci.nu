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

def "main candidate" [before: string, revision: string, output: path] {
  cd $project_dir
  hide-env --ignore-errors RELEASE_WORKSPACE_ROOT
  $"release=(release-candidate $before $revision)\n" | save --append --raw $output
}

def "main build" [revision: string] { build-assets $revision }

def "main publish" [revision: string] {
  cd $project_dir
  hide-env --ignore-errors RELEASE_WORKSPACE_ROOT
  if ($env.FORGEJO_REF? | default '') != 'refs/heads/main' or ($env.FORGEJO_EVENT_NAME? | default '') != push {
    fail 'publication is only allowed for a push to main'
  }
  let epoch = ^git show -s --format=%ct $revision | str trim
  # Reconcile remote state before building, and again immediately before writes.
  publication-state $revision $epoch | ignore
  build-assets $revision
  let plan = publication-state $revision $epoch
  if $plan.needs_tag {
    checked git [-c 'user.name=Forgejo Actions' -c 'user.email=actions@localhost' tag --annotate $plan.tag --message $"fjx v($plan.version)" $revision]
    checked $nu.current-exe [--no-config-file $release_script push-tag $plan.tag $revision]
  }
  let release_id = if $plan.needs_release {
    create-release $plan.version $plan.tag | into string
  } else { $plan.release_id }
  if ($plan.missing_assets | is-not-empty) {
    checked $nu.current-exe [--no-config-file $release_script upload-missing-assets $release_id $plan.missing_assets]
  }
  checked $nu.current-exe [--no-config-file $release_script verify-final $plan.version $revision $plan.tag]
  print $"Published and verified ($plan.tag)."
}

def main [] { fail 'usage: scripts/forgejo-ci.nu candidate BEFORE SHA OUTPUT | build SHA | publish SHA' }
