#!/usr/bin/env nu

use ../scripts/release.nu [publication-plan]
use ../scripts/package-release.nu [release-targets]
use ../scripts/release-smoke.nu [run-arm64-smoke-container]

const release_script = path self ../scripts/release.nu
const package_script = path self ../scripts/package-release.nu
const release_smoke_script = path self ../scripts/release-smoke.nu
const ci_release_script = path self ../scripts/ci-release.nu
const release_toml = path self ../release.toml
const release_targets = path self ../release-targets.toml
const release_containerfile = path self ../packaging/release.Containerfile
const release_workflow = path self ../../.forgejo/workflows/fjx.yml
const publish_workflow = path self ../../.forgejo/workflows/fjx-release.yml

def check [condition: bool, message: string] {
  if not $condition { error make {msg: $"release test: ($message)"} }
}

def expected-asset-names [version: string] {
  release-targets | each {|target|
    let archive = $"fjx-($version)-($target.triple).($target.archive)"
    [$archive $"($archive).sha256"]
  } | flatten
}

def fixture [version: string = 1.2.3] {
  let root = (^mktemp -d | str trim)
  mkdir ($root | path join fjx)
  $"version = \"($version)\"\n" | save --raw ($root | path join fjx release.toml)
  $"[package]\nname = \"fjx\"\nversion = \"($version)\"\n" | save --raw ($root | path join fjx Cargo.toml)
  $"version = 4\n\n[[package]]\nname = \"fjx\"\nversion = \"($version)\"\n" | save --raw ($root | path join fjx Cargo.lock)
  cp $release_targets ($root | path join fjx release-targets.toml)
  $root
}

def run-release [root: path, arguments: list<string>, extra_env: record = {}] {
  with-env ({RELEASE_WORKSPACE_ROOT: $root} | merge $extra_env) {
    do { ^nu --no-config-file $release_script ...$arguments } | complete
  }
}

def assert-versions [root: path, version: string] {
  let validated = (run-release $root [validate])
  check ($validated.exit_code == 0) $validated.stderr
  check (($validated.stdout | str trim) == $version) $"expected version ($version)"
}

def test-version-validation-and-tags [] {
  let root = (fixture)
  assert-versions $root 1.2.3
  let version = (run-release $root [version])
  check (($version.stdout | str trim) == '1.2.3') "version command returned the wrong value"
  check ((run-release $root [validate-tag fjx/v1.2.3]).exit_code == 0) "valid tag failed"
  check ((run-release $root [validate-tag fjx/v1.2.4]).exit_code != 0) "invalid tag passed"
  'version = "01.2.3"\n' | save --force --raw ($root | path join fjx release.toml)
  check ((run-release $root [validate]).exit_code != 0) "invalid semantic version passed"
  rm -rf $root
}

def test-dry-run-and-success [] {
  let root = (fixture)
  let before = (open --raw ($root | path join fjx release.toml))
  let dry = (run-release $root [prepare --dry-run minor])
  check ($dry.exit_code == 0) $dry.stderr
  check ($dry.stdout | str contains 'proposed_version=1.3.0') "dry-run plan is wrong"
  check ((open --raw ($root | path join fjx release.toml)) == $before) "dry run changed a file"
  let prepared = (run-release $root [prepare patch])
  check ($prepared.exit_code == 0) $prepared.stderr
  assert-versions $root 1.2.4
  rm -rf $root
}

def test-prepare-rollbacks [] {
  for move in [1 2 3] {
    let root = (fixture)
    let originals = [release.toml Cargo.toml Cargo.lock] | each {|file| open --raw ($root | path join fjx $file) }
    let failed = (run-release $root [prepare patch] {RELEASE_PREPARE_FAIL_AFTER_MOVE: ($move | into string)})
    check ($failed.exit_code != 0) $"failure after move ($move) succeeded"
    for item in ([release.toml Cargo.toml Cargo.lock] | enumerate) {
      check ((open --raw ($root | path join fjx $item.item)) == ($originals | get $item.index)) $"move ($move) did not roll back ($item.item)"
    }
    rm -rf $root
  }
  let root = (fixture)
  let originals = [release.toml Cargo.toml Cargo.lock] | each {|file| open --raw ($root | path join fjx $file) }
  let failed = (run-release $root [prepare patch] {RELEASE_PREPARE_FAIL_VALIDATION: 'true'})
  check ($failed.exit_code != 0) "final validation failure succeeded"
  for item in ([release.toml Cargo.toml Cargo.lock] | enumerate) {
    check ((open --raw ($root | path join fjx $item.item)) == ($originals | get $item.index)) $"final validation did not roll back ($item.item)"
  }
  rm -rf $root
}

def seed-binaries [] {
  let root = (^mktemp -d | str trim)
  for target in (release-targets) {
    let directory = ($root | path join $target.triple)
    mkdir $directory
    '#!/usr/bin/env nu\nprint fjx\n' | save --raw ($directory | path join $target.executable)
    if $target.os != windows { chmod 0755 ($directory | path join $target.executable) }
  }
  $root
}

def test-package-targets-and-seeded-assets [] {
  let output = (^mktemp -d | str trim)
  let duplicate = (^mktemp -d | str trim)
  let bins = (seed-binaries)
  let invalid = (with-env {FJX_PACKAGE_BIN_ROOT: $bins} { do { ^nu --no-config-file $package_script $output bad-target } | complete })
  check ($invalid.exit_code != 0) "invalid package target passed"
  with-env {FJX_PACKAGE_BIN_ROOT: $bins} {
    ^nu --no-config-file $package_script $output all --source-date-epoch 1700000000
    ^nu --no-config-file $package_script $duplicate all --source-date-epoch 1700000000
  }
  let version = (open --raw $release_toml | from toml | get version)
  for target in (release-targets) {
    let archive = $"fjx-($version)-($target.triple).($target.archive)"
    let contents = if $target.archive == tar.gz {
      ^tar -tzf ($output | path join $archive) | lines
    } else {
      ^unzip -Z1 ($output | path join $archive) | lines
    }
    check ($contents == [$target.executable]) $"bad contents for ($archive)"
    let sum = (do { cd $output; ^sha256sum --check $"($archive).sha256" } | complete)
    check ($sum.exit_code == 0) $"bad checksum for ($archive)"
    check ((open --raw ($output | path join $archive)) == (open --raw ($duplicate | path join $archive))) $"non-deterministic archive ($archive)"
  }
  check ((ls $output | where type == file | length) == 12) "six targets did not produce exactly twelve assets"
  rm -rf $output $duplicate $bins
}

def fake-runtime [asset_root: path, log: path] {
  let root = (^mktemp -d | str trim)
  let runtime = ($root | path join fake-runtime.nu)
  [
    '#!/usr/bin/env nu'
    'def --wrapped main [command: string, ...arguments] {'
    '  $"($command) ($arguments | str join (char space))\n" | save --append --raw $env.FAKE_RUNTIME_LOG'
    '  if $command == run {'
    '    let platform = ($arguments | get 3 | str replace / -)'
    '    ^cat | save --raw ($env.FAKE_RUNTIME_STDIN_ROOT | path join $platform)'
    '  }'
    '  match $command {'
    '    create => { print fake-container }'
    '    cp => {'
    '      let name = ($arguments | first | path basename)'
    '      cp ($env.FAKE_RUNTIME_ASSET_ROOT | path join $name) ($arguments | last)'
    '    }'
    '    _ => {}'
    '  }'
    '}'
    ''
  ] | str join (char nl) | save --raw $runtime
  chmod 0755 $runtime
  {root: $root, runtime: $runtime, asset_root: $asset_root, log: $log}
}

def fake-build-runtime [root: path, name: string, log: path] {
  let runtime = ($root | path join $name)
  [
    '#!/usr/bin/env nu'
    'def --wrapped main [...arguments] {'
    '  $arguments | to json --raw | save --append --raw $env.FAKE_BUILD_LOG'
    '  "\n" | save --append --raw $env.FAKE_BUILD_LOG'
    '  let target = ($arguments | where {|argument| $argument | str starts-with "TARGET=" } | first | str replace "TARGET=" "")'
    '  let executable = ($arguments | where {|argument| $argument | str starts-with "EXECUTABLE=" } | first | str replace "EXECUTABLE=" "")'
    '  let destination = ($arguments | where {|argument| $argument | str starts-with "type=local,dest=" } | first | str replace "type=local,dest=" "")'
    '  if ($env.FAKE_BUILD_FAIL_TARGET? | default "") == $target { exit 17 }'
    '  mkdir $destination'
    '  $"#!/usr/bin/env nu\nprint ($target)\n" | save --raw ($destination | path join $executable)'
    '}'
    ''
  ] | str join (char nl) | save --raw $runtime
  chmod 0755 $runtime
  $runtime
}

def exercise-arm64-smoke [states: list<string>, timeout: duration = 2sec, poll_interval: duration = 1ms, fail_copy: string = ''] {
  let root = (^mktemp -d | str trim)
  let runtime = ($root | path join fake-smoke-runtime.nu)
  let calls_path = ($root | path join calls.jsonl)
  let states_path = ($root | path join states.txt)
  let index_path = ($root | path join index.txt)
  ($states | str join (char nl)) | save --raw $states_path
  '0' | save --raw $index_path
  [
    '#!/usr/bin/env nu'
    'def --wrapped main [command: string, ...arguments] {'
    '  ({command: $command, arguments: $arguments} | to json --raw) + (char nl) | save --append --raw $env.FAKE_SMOKE_CALLS'
    '  match $command {'
    '    cp => { if $env.FAKE_SMOKE_FAIL_COPY == ($arguments | first | path basename) { exit 12 } }'
    '    start => { print fake-smoke-container }'
    '    inspect => {'
    '      let states = (open --raw $env.FAKE_SMOKE_STATES | lines)'
    '      let index = (open --raw $env.FAKE_SMOKE_INDEX | into int)'
    '      let selected = ([$index (($states | length) - 1)] | math min)'
    '      (($index + 1) | into string) | save --force --raw $env.FAKE_SMOKE_INDEX'
    '      print --no-newline ($states | get $selected)'
    '    }'
    '    logs => { print fake-fjx-help }'
    '    rm => {}'
    '    _ => { error make {msg: $"unexpected fake runtime command ($command)"} }'
    '  }'
    '}'
    ''
  ] | str join (char nl) | save --raw $runtime
  chmod 0755 $runtime
  let outcome = (with-env {
    FAKE_SMOKE_CALLS: $calls_path
    FAKE_SMOKE_STATES: $states_path
    FAKE_SMOKE_INDEX: $index_path
    FAKE_SMOKE_FAIL_COPY: $fail_copy
  } {
    try {
      run-arm64-smoke-container $runtime fake-smoke-container linux/arm64 /fake/qemu-aarch64 /fake/fjx $timeout $poll_interval
      {ok: true, error: ''}
    } catch {|error|
      {ok: false, error: $error.msg}
    }
  })
  let calls = (open --raw $calls_path | lines | each { from json })
  rm -rf $root
  {outcome: $outcome, calls: $calls}
}

def check-smoke-cleanup [case: record, name: string] {
  check (($case.calls | last | get command) == rm) $"($name) did not finish with container cleanup"
  check (($case.calls | last | get arguments) == [--force fake-smoke-container]) $"($name) cleanup was not forced and scoped"
}

def test-arm64-smoke-state-machine [] {
  let success = (exercise-arm64-smoke [
    '[{"State":{"Status":"running","ExitCode":0}}]'
    '[{"State":{"Status":"exited","ExitCode":0}}]'
  ])
  check $success.outcome.ok $success.outcome.error
  check (($success.calls | get command) == [cp cp start inspect inspect logs rm]) "successful ARM64 smoke did not prepare, poll, read logs, and clean up"
  check-smoke-cleanup $success success

  let nonzero = (exercise-arm64-smoke ['[{"State":{"Status":"exited","ExitCode":7}}]'])
  check (not $nonzero.outcome.ok) "nonzero ARM64 smoke exit passed"
  check ($nonzero.outcome.error | str contains 'exit code 7') "nonzero ARM64 smoke failure omitted its exit code"
  check (($nonzero.calls | get command) == [cp cp start inspect logs rm]) "nonzero ARM64 smoke did not retrieve logs before cleanup"
  check-smoke-cleanup $nonzero nonzero

  let copy_failed = (exercise-arm64-smoke ['[{"State":{"Status":"exited","ExitCode":0}}]'] 2sec 1ms fjx)
  check (not $copy_failed.outcome.ok) "ARM64 smoke copy failure passed"
  check ($copy_failed.outcome.error | str contains 'could not prepare') "ARM64 smoke copy failure omitted its reason"
  check (($copy_failed.calls | get command) == [cp cp rm]) "ARM64 smoke copy failure was not cleaned up exactly once"
  check-smoke-cleanup $copy_failed copy-failure

  let invalid = (exercise-arm64-smoke ['{'])
  check (not $invalid.outcome.ok) "invalid ARM64 smoke inspection JSON passed"
  check ($invalid.outcome.error | str contains 'invalid ARM64 Linux smoke state') $"invalid inspection failure omitted its reason: ($invalid.outcome.error)"
  check-smoke-cleanup $invalid invalid-json

  let unexpected = (exercise-arm64-smoke ['[{"State":{"Status":"paused","ExitCode":0}}]'])
  check (not $unexpected.outcome.ok) "unexpected ARM64 smoke status passed"
  check ($unexpected.outcome.error | str contains 'unexpected status paused') "unexpected-state failure omitted its status"
  check-smoke-cleanup $unexpected unexpected-status

  let timed_out = (exercise-arm64-smoke ['[{"State":{"Status":"running","ExitCode":0}}]'] 2ms 5ms)
  check (not $timed_out.outcome.ok) "hung ARM64 smoke passed"
  check ($timed_out.outcome.error | str contains 'timed out after 2ms') "ARM64 smoke timeout omitted its bound"
  check ('logs' in ($timed_out.calls | get command)) "ARM64 smoke timeout did not retrieve logs"
  check-smoke-cleanup $timed_out timeout
}

def test-build-runtime-command-shapes [] {
  let root = (^mktemp -d | str trim)
  let log = ($root | path join runtime.log)
  let version = (open --raw $release_toml | from toml | get version)
  for runtime_name in [podman docker] {
    let runtime = (fake-build-runtime $root $runtime_name $log)
    let output = ($root | path join $"output-($runtime_name)")
    with-env {FAKE_BUILD_LOG: $log} {
      ^nu --no-config-file $package_script build $output --runtime $runtime --source-date-epoch 1700000000
    }
    check (($output | path join $"fjx-($version)-x86_64-unknown-linux-gnu.tar.gz") | path exists) $"($runtime_name) build did not package assets"
  }
  let invocations = (open --raw $log | lines | each { from json })
  check (($invocations | length) == 12) "fake runtimes did not build all six targets"
  for invocation in ($invocations | first 6) {
    check (($invocation | first) == build) "Podman did not use its native build command"
    check ('buildx' not-in $invocation) "Podman unexpectedly used Docker Buildx"
  }
  for invocation in ($invocations | last 6) {
    check (($invocation | first 2) == [buildx build]) "Docker did not use Buildx"
  }
  rm -rf $root
}

def test-smoke-requires-wine [] {
  let assets = (^mktemp -d | str trim)
  let version = (open --raw $release_toml | from toml | get version)
  let missing_wine = (do { ^nu --no-config-file $package_script verify $assets $version --smoke } | complete)
  check ($missing_wine.exit_code != 0) "smoke verification silently skipped the Wine target"
  check ($missing_wine.stderr | str contains '--wine is required') "missing-Wine failure omitted its reason"
  check (not ($missing_wine.stderr | str contains 'release asset inventory mismatch')) "missing-Wine validation ran after asset verification"
  rm -rf $assets
}

def test-default-runtime-and-asset-checks [] {
  let bins = (seed-binaries)
  let assets = (^mktemp -d | str trim)
  with-env {FJX_PACKAGE_BIN_ROOT: $bins} { ^nu --no-config-file $package_script $assets all }
  let version = (open --raw $release_toml | from toml | get version)
  let wrong_format = (do { ^nu --no-config-file $package_script verify $assets $version } | complete)
  check ($wrong_format.exit_code != 0) "target-aware executable validation accepted scripts"
  check ($wrong_format.stderr | str contains 'wrong executable format') "format failure omitted its reason"
  touch ($assets | path join unexpected.txt)
  let extra = (do { ^nu --no-config-file $package_script verify $assets $version } | complete)
  check ($extra.exit_code != 0) "extra release asset passed"
  check ($extra.stderr | str contains 'release asset inventory mismatch') "extra-asset failure omitted the inventory mismatch"
  check ($extra.stderr | str contains 'unexpected.txt') "extra-asset failure omitted the inventory"
  rm ($assets | path join unexpected.txt)
  'bad' | save --append --raw ($assets | path join $"fjx-($version)-x86_64-unknown-linux-gnu.tar.gz")
  let bad = (do { ^nu --no-config-file $package_script verify $assets $version } | complete)
  check ($bad.exit_code != 0) "bad asset checksum passed"
  rm -rf $bins $assets
}

def test-publish-planning [] {
  let root = (fixture)
  with-env {RELEASE_WORKSPACE_ROOT: $root} {
    let absent = (publication-plan abc abc 2 '123')
    check $absent.publish "absent tag did not plan publication"
    check ($absent.tag == 'fjx/v1.2.3') "publication tag is wrong"
    let present = (publication-plan abc abc 0 '123')
    check (not $present.publish) "present tag planned publication"
    check ((try { publication-plan abc def 2 '123'; false } catch { true })) "revision mismatch passed"
    check ((try { publication-plan abc abc 1 '123'; false } catch { true })) "remote lookup error passed"
  }
  rm -rf $root
}

def test-release-metadata-mutations [] {
  let root = fixture
  let fake_bin = $root | path join bin
  let asset_names = (expected-asset-names 1.2.3 | each {|name| $'"($name)"' } | str join ' ')
  mkdir $fake_bin
  [
    '#!/usr/bin/env -S nu --no-config-file'
    'def --wrapped main [...args: string] {'
    '  if ($args | take 3) == [remote get-url origin] { print "https://forge.example/dalton/monolith.git"; return }'
    '  if ($args | first) == "rev-parse" { print $env.FAKE_REVISION; return }'
    '  print "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb refs/tags/fjx/v1.2.3"'
    '  print $"($env.FAKE_REVISION) refs/tags/fjx/v1.2.3^{}"'
    '}'
  ] | str join (char nl) | save --raw ($fake_bin | path join git)
  [
    '#!/usr/bin/env -S nu --no-config-file'
    'def --wrapped main [...args: string] {'
    '  let output_index = $args | enumerate | where item == "--output" | first | get index'
    '  let output = $args | get ($output_index + 1)'
    '  let state = $env.FAKE_METADATA'
    '  let tag = if $state == "tag" { "fjx/v9.9.9" } else { "fjx/v1.2.3" }'
    '  let name = if $state == "title" { "wrong" } else { "fjx (1.2.3)" }'
    '  let body = if $state == "body" { "wrong" } else { "" }'
    $'  let assets = [($asset_names)] | each {|name| {name: $name} }'
    '  {id: 42, tag_name: $tag, name: $name, body: $body, draft: ($state == "draft"), prerelease: ($state == "prerelease"), assets: $assets} | to json | save --raw $output'
    '  print --no-newline 200'
    '}'
  ] | str join (char nl) | save --raw ($fake_bin | path join curl)
  chmod 0755 ($fake_bin | path join git) ($fake_bin | path join curl)
  let revision = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
  for state in [title body draft prerelease tag] {
    let output = $root | path join $"metadata-($state)"
    touch $output
    let result = run-release $root [publish-plan $revision 123 $output] {
      PATH: ($env.PATH | prepend $fake_bin)
      FORGEJO_API_URL: 'https://forge.example/api/v1'
      FORGEJO_REPOSITORY: 'dalton/monolith'
      FORGEJO_TOKEN: 'fixture'
      FAKE_REVISION: $revision
      FAKE_METADATA: $state
    }
    check ($result.exit_code != 0) $"release metadata mutation passed: ($state)"
  }
  rm -rf $root
}

def test-authenticated-release-origin [] {
  let root = fixture
  let project = $root | path join fjx
  let assets = $project | path join publish-assets
  let payload = $root | path join payload
  let fake_bin = $root | path join bin
  let curl_log = $root | path join curl.log
  let foreign_log = $root | path join foreign.log
  let expected_names = (expected-asset-names 1.2.3)
  let names_literal = ($expected_names | each {|name| $'"($name)"' } | str join ' ')
  let name_map = ($expected_names | enumerate | reduce --fold {} {|entry, map|
    $map | insert (($entry.index + 101) | into string) $entry.item
  } | to json --raw)
  mkdir $assets $payload $fake_bin
  touch $curl_log $foreign_log
  '#!/usr/bin/env nu\nprint fixture\n' | save --raw ($payload | path join fjx)
  '#!/usr/bin/env nu\nprint fixture\n' | save --raw ($payload | path join fjx.exe)
  chmod 0755 ($payload | path join fjx) ($payload | path join fjx.exe)
  for target in (release-targets) {
    let archive = $"fjx-1.2.3-($target.triple).($target.archive)"
    if $target.archive == tar.gz {
      ^tar -czf ($assets | path join $archive) -C $payload $target.executable
    } else {
      do { cd $payload; ^zip -X -q ($assets | path join $archive) $target.executable }
    }
    do { cd $assets; ^sha256sum $archive } | save --raw ($assets | path join $"($archive).sha256")
  }
  [
    '#!/usr/bin/env -S nu --no-config-file'
    'def --wrapped main [...args: string] {'
    '  if ($args | take 3) == [remote get-url origin] { print "https://forge.example/dalton/monolith.git"; return }'
    '  print "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb refs/tags/fjx/v1.2.3"'
    '  print $"($env.FAKE_REVISION) refs/tags/fjx/v1.2.3^{}"'
    '}'
  ] | str join (char nl) | save --raw ($fake_bin | path join git)
  [
    '#!/usr/bin/env -S nu --no-config-file'
    'def --wrapped main [...args: string] {'
    '  ($args | to json --raw) + (char nl) | save --append --raw $env.FAKE_CURL_LOG'
    '  let output_index = $args | enumerate | where item == "--output" | first | get index'
    '  let output = $args | get ($output_index + 1)'
    '  let target = $args | last'
    '  if ($target | str contains "evil.example") and ($args | str join " " | str contains "Authorization: token") { "token disclosed" | save --append --raw $env.FAKE_FOREIGN_LOG }'
    '  if $target =~ "/releases/assets/[0-9]+$" {'
    '    if $env.FAKE_RELEASE_STATE == "redirect" { "redirect" | save --raw $output; print --no-newline 302; return }'
    $"    let names = '($name_map)' | from json"
    '    cp ($env.FAKE_ASSET_ROOT | path join ($names | get ($target | path basename))) $output'
    '    print --no-newline 200'
    '    return'
    '  }'
    $'  let names = [($names_literal)]'
    '  let origin = match $env.FAKE_RELEASE_STATE { "foreign" => "https://evil.example", "http-asset" => "http://forge.example", _ => "https://forge.example" }'
    '  let assets = $names | enumerate | each {|entry|'
    '    let base = {name: $entry.item, browser_download_url: $"($origin)/assets/($entry.item)"}'
    '    if $env.FAKE_RELEASE_STATE == "missing-id" and $entry.index == 0 { $base } else { $base | merge {id: (if $env.FAKE_RELEASE_STATE == "invalid-id" and $entry.index == 0 { 0 } else { $entry.index + 101 })} }'
    '  }'
    '  {id: 42, tag_name: "fjx/v1.2.3", name: "fjx (1.2.3)", body: "", draft: false, prerelease: false, assets: $assets} | to json | save --force --raw $output'
    '  print --no-newline 200'
    '}'
  ] | str join (char nl) | save --raw ($fake_bin | path join curl)
  chmod 0755 ($fake_bin | path join git) ($fake_bin | path join curl)
  let revision = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
  let environment = {
    PATH: ($env.PATH | prepend $fake_bin)
    FORGEJO_API_URL: 'https://forge.example/api/v1'
    FORGEJO_REPOSITORY: 'dalton/monolith'
    FORGEJO_TOKEN: 'origin-fixture-secret'
    FAKE_REVISION: $revision
    FAKE_ASSET_ROOT: $assets
    FAKE_CURL_LOG: $curl_log
    FAKE_FOREIGN_LOG: $foreign_log
  }

  let valid = run-release $root [verify-final 1.2.3 $revision fjx/v1.2.3] ($environment | merge {FAKE_RELEASE_STATE: valid})
  check ($valid.exit_code == 0) $"exact same-origin assets failed verification: ($valid.stderr)"
  let calls = open --raw $curl_log
  check ($calls | str contains 'https://forge.example/api/v1/repos/dalton/monolith/releases/assets/101') "asset bytes were not fetched through the verified Forgejo asset ID endpoint"
  check (not ($calls | str contains '"--location"')) "authenticated asset downloads permit redirects"

  for state in [foreign http-asset redirect missing-id invalid-id] {
    '' | save --force --raw $curl_log
    '' | save --force --raw $foreign_log
    let result = run-release $root [verify-final 1.2.3 $revision fjx/v1.2.3] ($environment | merge {FAKE_RELEASE_STATE: $state})
    check ($result.exit_code != 0) $"unsafe asset fixture passed: ($state)"
    check (not ($result.stderr | str contains 'origin-fixture-secret')) $"token appeared in failure output for ($state)"
    let unsafe_calls = open --raw $curl_log
    check (not ($unsafe_calls | str contains 'https://evil.example')) $"authenticated request reached a foreign origin for ($state)"
    check (not ($unsafe_calls | str contains 'http://forge.example/assets')) $"authenticated request reached an HTTP asset URL for ($state)"
    check ((open --raw $foreign_log) == '') $"token reached a foreign origin for ($state)"
    if $state == redirect {
      check ($unsafe_calls | str contains '/releases/assets/101') "redirect fixture did not reach the same-origin asset endpoint"
      check (not ($unsafe_calls | str contains '"--location"')) "redirect fixture was followed with authentication"
    }
  }

  for api in ['http://forge.example/api/v1' 'https://user@forge.example/api/v1' 'https://forge.example:bad/api/v1'] {
    '' | save --force --raw $curl_log
    let result = run-release $root [verify-final 1.2.3 $revision fjx/v1.2.3] ($environment | merge {FORGEJO_API_URL: $api, FAKE_RELEASE_STATE: valid})
    check ($result.exit_code != 0) $"unsafe Forgejo API URL passed: ($api)"
    check ((open --raw $curl_log) == '') $"authenticated API request started for unsafe URL: ($api)"
    check (not ($result.stderr | str contains 'origin-fixture-secret')) $"token appeared in unsafe API URL failure: ($api)"
  }
  rm -rf $root
}

def test-release-build-contract [] {
  let targets = (release-targets)
  check (($targets | get triple) == [x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu x86_64-unknown-linux-musl aarch64-unknown-linux-musl aarch64-apple-darwin x86_64-pc-windows-gnu]) "authoritative target inventory is wrong"
  check ('x86_64-apple-darwin' not-in ($targets | get triple)) "x86_64 macOS is present"
  check (($targets | where os == macos | get runtime | first) == none) "macOS claims runtime validation"
  check (($targets | where os == windows | first | select archive executable runtime) == {archive: zip, executable: fjx.exe, runtime: wine}) "Windows policy is wrong"
  for target in ($targets | where triple =~ 'musl$') {
    let static = $"ELF 64-bit LSB executable, ($target.arch | if $in == x86_64 { 'x86-64' } else { 'ARM aarch64' }), statically linked"
    let static_pie = $"ELF 64-bit LSB pie executable, ($target.arch | if $in == x86_64 { 'x86-64' } else { 'ARM aarch64' }), static-pie linked"
    let dynamic = $"ELF 64-bit LSB pie executable, ($target.arch | if $in == x86_64 { 'x86-64' } else { 'ARM aarch64' }), dynamically linked"
    check ($static =~ $target.file_pattern) $"musl pattern rejects statically linked ($target.triple)"
    check ($static_pie =~ $target.file_pattern) $"musl pattern rejects static-pie linked ($target.triple)"
    check (not ($dynamic =~ $target.file_pattern)) $"musl pattern accepts dynamically linked ($target.triple)"
  }
  let windows = ($targets | where os == windows | first)
  let windows_x86_64 = 'PE32+ executable for MS Windows 6.00 (console), x86-64, 6 sections'
  let windows_x86 = 'PE32 executable for MS Windows 6.00 (console), Intel 80386, 5 sections'
  check ($windows_x86_64 =~ $windows.file_pattern) "Windows pattern rejects the pinned runner's x86-64 file description"
  check (not ($windows_x86 =~ $windows.file_pattern)) "Windows pattern accepts a 32-bit executable"
  let containerfile = (open --raw $release_containerfile)
  check ($containerfile | str contains 'ghcr.io/rust-cross/cargo-zigbuild:0.23.3@sha256:76ed3823d8cd9d8b409b10f9c4cda292b0c8699175ea4c0a2d541775c8184d2b') "cross builder is not digest-pinned"
  check ($containerfile | str contains 'test "$(/usr/local/cargo/bin/cargo-zigbuild --version)" = "cargo-zigbuild 0.23.3"') "cargo-zigbuild executable version is not asserted"
  check ($containerfile | str contains 'test "$(zig version)" = 0.16.0') "Zig version is not asserted"
  check ($containerfile | str contains 'test "$SDKROOT" = /opt/MacOSX11.3.sdk') "macOS SDK version is not asserted"
  check ($containerfile | str contains 'for attempt in 1 2 3 4 5; do') "Rust toolchain download does not have a bounded retry loop"
  check ($containerfile | str contains 'rustup toolchain install 1.97.1 --profile minimal && break;') "Rust toolchain retry does not stop after a successful install"
  check ($containerfile | str contains 'if [ "$attempt" = 5 ]; then exit 1; fi;') "Rust toolchain retry does not fail after its fifth attempt"
  check ($containerfile | str contains 'sleep "$attempt";') "Rust toolchain retry does not back off between attempts"
  check (not ($containerfile | str contains 'cargo install --list')) "unavailable Cargo install metadata probe returned"
  check (not ($containerfile | str contains 'cargo zigbuild --version')) "unsupported cargo-zigbuild version probe returned"
  check ($containerfile | str contains 'cargo zigbuild --locked --release --target "$TARGET"') "cross build does not honor its exact target"
  check (not ($containerfile | str contains ':latest')) "release container retains a mutable latest tag"
  let package_script_text = (open --raw $package_script)
  check ($package_script_text | str contains "const linux_amd64_smoke_image = 'docker.io/library/debian:bookworm-slim@sha256:88200866dfff7ea7f5cbcb6ec7c8a701889efe6fe859fe64d6990e4b07ea4171'") "AMD64 Linux smoke image is not digest-pinned"
  check ($package_script_text | str contains "const linux_arm64_smoke_image = 'docker.io/library/debian:bookworm-slim@sha256:6bd27d44e6c32a66bbd72d7cb2b76a8ae3497ec2e5274a81abd1b37f6013fa1f'") "ARM64 Linux smoke image is not digest-pinned"

  let check_workflow = (open --raw $release_workflow)
  let publish_workflow_text = (open --raw $publish_workflow)
  let workflow = [$check_workflow $publish_workflow_text] | str join "\n"
  let parsed = (open $release_workflow)
  let publish_parsed = (open $publish_workflow)
  check ($parsed.concurrency | get "cancel-in-progress") "check workflow does not cancel superseded work"
  check (($publish_parsed.concurrency | get "cancel-in-progress") == false) "release workflow may cancel publication"
  check (not ($workflow | str contains 'DOCKER_BUILD_SUMMARY')) "direct Buildx workflow retains an action-only summary flag"
  check (not ($workflow | str contains 'DOCKER_BUILD_RECORD_UPLOAD')) "direct Buildx workflow retains an action-only record flag"

  check (($parsed.jobs | columns) == [check]) "project workflow must contain only its check job"
  check (($publish_parsed.jobs | columns) == [publish]) "release workflow must contain only its publish job"
  let check_job = $parsed.jobs.check
  check ($check_job.name == 'Check') "check has the wrong display name"
  check ($check_job.runs-on == 'ci-node-22') "pull-request check does not use the CI runner"
  check ($check_job.container | str contains '@sha256:') "check container is not digest pinned"
  check (($parsed | get on | columns) == [pull_request workflow_dispatch]) "check workflow has unexpected triggers"
  let check_text = $check_job | to yaml
  check ($check_text | str contains 'scripts/check.nu fjx') "check workflow does not run the project gate"
  check (not ($check_text | str contains 'build-package-smoke')) "pull request still builds release assets"
  check (not ($check_text | str contains 'release-node-22')) "pull request can select the release runner"
  check (not ($check_text | str contains 'FORGEJO_TOKEN')) "pull-request check has a Forgejo token context"
  let publish = $publish_parsed.jobs.publish
  check (($publish.needs? | default null) == null) "publish unexpectedly depends on a pull-request job"
  check (($publish_parsed | get on | columns) == [workflow_dispatch]) "release workflow is not manual"
  check ($publish.steps.0.name == 'Require main') "release does not reject non-main refs first"
  check ($publish.steps.0.run | str contains 'refs/heads/main') "release main-ref guard is missing"
  check ($publish.runs-on == 'release-node-22') "release does not use the release runner"
  let publish_checkout = $publish.steps | where name == 'Check out exact main revision without credentials' | first
  check ($publish_checkout.env.EXPECTED_REVISION == '${{ forgejo.sha }}') "publish checkout does not select the triggering main revision"
  check ($publish_checkout.run | str contains 'git fetch --depth=1 origin "$EXPECTED_REVISION"') "publish checkout is not an exact unauthenticated fetch"
  check ($publish_checkout.run | str contains 'test "$(git rev-parse HEAD)" = "$EXPECTED_REVISION"') "publish checkout does not verify the fetched revision"
  let publish_text = $publish_workflow_text | split row "\n  publish:\n" | last
  check ($publish_text | str contains 'title: fjx (${{ steps.release.outputs.version }})') "Forgejo action title does not match the driver metadata contract"
  check (($publish_text | split row 'fjx/scripts/release.nu push-tag $env.RELEASE_TAG $env.FORGEJO_SHA' | length) == 2) "publish does not delegate the tag push exactly once"
  for bypass in ['http.extraHeader' ' push origin '] {
    check (not ($publish_text | str contains $bypass)) $"publish bypasses the release tag driver through: ($bypass)"
  }

  let driver_call = 'fjx/scripts/ci-release.nu build-package-smoke'
  check (($workflow | split row $driver_call | length) == 2) "release does not call the package driver exactly once"
  let driver = (open --raw $ci_release_script)
  for required in [
    'package-release.nu) build $asset_dir --runtime $runtime --source-date-epoch'
    'verify-assets $asset_dir $version --runtime=$runtime --wine=$wine --smoke'
    'Wine is required to smoke the Windows release target'
  ] {
    check ($driver | str contains $required) $"six-target release driver lacks: ($required)"
  }
  let asset_verifications = $driver
    | lines
    | where {|line| $line | str trim | str starts-with 'verify-assets ' }
  check (($asset_verifications | length) == 1) "release driver must verify exported assets exactly once"
  check ((($asset_verifications | first) | str trim) == 'verify-assets $asset_dir $version --runtime=$runtime --wine=$wine --smoke') "release driver must smoke the complete six-target asset set"
  let release_driver = open --raw $release_script
  for required in ['def "main push-tag"' '^git remote get-url origin' 'http.followRedirects=false' 'git cat-file -t' 'Forgejo token is required to push a release tag' 'def "main upload-missing-assets"' 'def "main verify-final"' 'must be greater than prior released version' 'browser_download_url' 'releases/assets/($asset_id)' "--proto '=https' --max-redirs 0" 'cmp --silent' 'does not match the locally checked output' 'draft: false' 'prerelease: false'] {
    check ($release_driver | str contains $required) $"release publication boundary lacks: ($required)"
  }
  check ($workflow | str contains 'Upload only missing release assets') "publish cannot resume missing asset uploads"
  check ($workflow | str contains 'Verify final publication state') "publish lacks final Forgejo verification"
  check ($publish_workflow_text | str contains 'scripts/ci.nu verify-runner --docker') "release does not verify Docker"
  check ($publish_workflow_text | str contains 'scripts/ci.nu verify-runner --docker --buildx') "release does not verify Buildx"
  check ($publish_workflow_text | str contains 'docker/setup-buildx-action@8d2750c68a42422c14e847fe6c8ac0403b4cbd6f') "release lacks pinned Buildx setup"
  check ($publish_workflow_text | str contains 'version: https://github.com/docker/buildx.git#bac71def78b077ee6a2607119f88e291861b18ac') "release lacks the exact Buildx source commit"
  check (not ($workflow | str contains 'docker/setup-qemu-action@')) "fjx release depends on host binfmt emulation instead of its explicit build-platform emulator"
  check (not ($workflow | str contains 'docker/build-push-action@')) "fjx still uses the Forgejo-incompatible Buildx action"

  let version = (open --raw $release_toml | from toml | get version)
  let wrong_version = (do { ^nu --no-config-file $ci_release_script build-package-smoke 0.0.0 123 } | complete)
  check ($wrong_version.exit_code != 0) "release driver accepted the wrong version"
  let bad_epoch = (do { ^nu --no-config-file $ci_release_script build-package-smoke $version invalid } | complete)
  check ($bad_epoch.exit_code != 0) "release driver accepted an invalid source date epoch"
}

def test-ci-build-arguments [] {
  let version = (open --raw $release_toml | from toml | get version)
  let root = (fixture $version)
  let project = ($root | path join fjx)
  let scripts = ($project | path join scripts)
  mkdir $scripts
  cp $ci_release_script $package_script $release_script $release_smoke_script $scripts
  cp $release_targets ($project | path join release-targets.toml)

  let log = ($root | path join runtime.jsonl)
  let runtime = (fake-build-runtime $root argument-logger.nu $log)

  let result = with-env {RELEASE_WORKSPACE_ROOT: $root, FAKE_BUILD_LOG: $log, FAKE_BUILD_FAIL_TARGET: x86_64-pc-windows-gnu} {
    do { ^nu --no-config-file ($scripts | path join ci-release.nu) build-package-smoke $version 1 --runtime $runtime } | complete
  }
  check ($result.exit_code != 0) "release driver unexpectedly continued after the injected final-target build failure"

  let calls = open --raw $log | lines | each { from json }
  check (($calls | length) == 7) "release driver did not retry the injected final-target build failure exactly once"
  check (($calls | each {|arguments| $arguments | take 2 } | all {|prefix| $prefix == [buildx build] })) "release driver did not invoke Docker-compatible Buildx for every target"
  let build_targets = ($calls | each {|arguments|
    $arguments | where {|argument| $argument | str starts-with 'TARGET=' } | first | str replace 'TARGET=' ''
  })
  check (($build_targets | drop 1) == (release-targets | get triple)) "release driver build order diverges from the authoritative target manifest"
  check (($build_targets | last 2) == [x86_64-pc-windows-gnu x86_64-pc-windows-gnu]) "release driver retried the wrong target"
  check (($calls | all {|arguments| ($arguments | last) == $project })) "project build context did not reach every target build boundary"
  rm -rf $root
}

test-version-validation-and-tags
test-dry-run-and-success
test-prepare-rollbacks
test-package-targets-and-seeded-assets
test-default-runtime-and-asset-checks
test-build-runtime-command-shapes
test-arm64-smoke-state-machine
test-smoke-requires-wine
test-publish-planning
test-release-metadata-mutations
test-authenticated-release-origin
test-release-build-contract
test-ci-build-arguments
print 'fjx release tests passed'
