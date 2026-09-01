#!/usr/bin/env nu

use package-release.nu [verify-assets]
use release.nu [validate-versions]

const script_dir = path self .

def fail [message: string] {
  error make {msg: $"fjx CI release: ($message)"}
}

def require-success [label: string] {
  if $env.LAST_EXIT_CODE != 0 {
    fail $"($label) failed with exit code ($env.LAST_EXIT_CODE)"
  }
}

export def build-package-smoke [version: string, source_date_epoch: string, --runtime: string = docker] {
  let expected = (validate-versions)
  if $version != $expected {
    fail $"release version ($version) does not match project version ($expected)"
  }
  if $source_date_epoch !~ '^[1-9][0-9]*$' {
    fail "source date epoch must be a positive integer"
  }

  let project_dir = ($script_dir | path dirname)
  let asset_dir = ($project_dir | path join publish-assets)
  if ($asset_dir | path exists) {
    fail "release output directories must not already exist"
  }
  ^nu --no-config-file ($script_dir | path join package-release.nu) build $asset_dir --runtime $runtime --source-date-epoch ($source_date_epoch | into int)
  require-success "six-target release build"

  let wine = (^which wine | str trim)
  if ($wine | is-empty) { fail "Wine is required to smoke the Windows release target" }
  verify-assets $asset_dir $version --runtime=$runtime --wine=$wine --smoke
}

def "main build-package-smoke" [version: string, source_date_epoch: string, --runtime: string = docker] {
  build-package-smoke $version $source_date_epoch --runtime=$runtime
}

def main [] {
  fail "usage: fjx/scripts/ci-release.nu build-package-smoke VERSION SOURCE_DATE_EPOCH [--runtime RUNTIME]"
}
