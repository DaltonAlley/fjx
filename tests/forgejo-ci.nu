#!/usr/bin/env nu

use ../scripts/forgejo-ci.nu [release-candidate]

def check [condition: bool, message: string] {
  if not $condition { error make {msg: $"Forgejo CI test: ($message)"} }
}

def --wrapped git [...arguments: string] {
  let result = do { ^git ...$arguments } | complete
  if $result.exit_code != 0 { error make {msg: $result.stderr} }
  $result.stdout | str trim
}

def commit-version [version: string] {
  $"version = '($version)'\n" | save --force --raw release.toml
  $"[package]\nname = 'fjx'\nversion = '($version)'\n" | save --force --raw Cargo.toml
  $"version = 4\n[[package]]\nname = 'fjx'\nversion = '($version)'\n" | save --force --raw Cargo.lock
  git add . | ignore
  git -c user.name=Fixture -c user.email=fixture@localhost commit --allow-empty -m $version | ignore
  git rev-parse HEAD
}

def main [] {
  let root = mktemp -d
  let project = $root | path join fjx
  mkdir $project
  try {
    with-env {RELEASE_WORKSPACE_ROOT: $root} {
      cd $project
      git init | ignore
      let before = commit-version 1.2.9
      let unchanged = commit-version 1.2.9
      check (not (release-candidate $before $unchanged)) 'ordinary commits must not publish'
      let increased = commit-version 1.2.10
      check (release-candidate $before $increased) 'numeric patch increases must publish'
      check (not (release-candidate ('0' | fill --width 40 --character 0) $increased)) 'initial imports must not publish'
      for arguments in [[$before $unchanged] [bad $increased] [($increased | str replace 'a' 'z') bad]] {
        let rejected = try { release-candidate $arguments.0 $arguments.1 | ignore; false } catch { true }
        check $rejected 'wrong or invalid revisions must fail'
      }
      let decreased = commit-version 1.2.8
      let rejected = try { release-candidate $increased $decreased | ignore; false } catch { true }
      check $rejected 'version decreases must fail'
      let major = commit-version 2.0.0
      check (release-candidate $decreased $major) 'major increases must publish'
      "version = '9.9.9'\n" | save --force --raw release.toml
      let mismatch = try { release-candidate $decreased $major | ignore; false } catch { true }
      check $mismatch 'inconsistent version files must fail'
    }
  } catch {|error|
    rm --recursive --force $root
    error make $error.raw
  }
  rm --recursive --force $root
  print 'Forgejo CI tests passed'
}
