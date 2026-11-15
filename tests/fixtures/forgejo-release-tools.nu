# Simulated external tools for the real candidate/build/publish entry points.
# Git ancestry, packaging, checksums, comparisons and local smoke calls stay real.
def log [event: string, value: string = ''] {
  {event: $event, value: $value} | to json --raw | $in + (char nl) | save --append --raw ($env.FAKE_STATE | path join calls)
}

export def fake-git [args: list<string>] {
  let tag = $env.FAKE_STATE | path join tag
  if ($args | take 3) == [remote get-url origin] { print 'https://forge.example/forgejo/dalton/fjx.git'; return }
  if $args.0 == ls-remote {
    if ($tag | path exists) { open --raw $tag | print --no-newline }
    return
  }
  if $args.0 == push {
    let reference = $args | last
    let object = ^$env.FAKE_REAL_GIT rev-parse $reference | str trim
    let revision = ^$env.FAKE_REAL_GIT rev-parse $"($reference)^{}" | str trim
    $"($object) ($reference)\n($revision) ($reference)^{}\n" | save --force --raw $tag
    log push-tag
    return
  }
  ^$env.FAKE_REAL_GIT ...$args
  exit $env.LAST_EXIT_CODE
}

export def fake-cargo [args: list<string>] {
  for name in [FJX_TOKEN FORGEJO_TOKEN GITEA_TOKEN GITHUB_TOKEN ACTIONS_RUNTIME_TOKEN] {
    if $name in ($env | columns) { error make {msg: 'job credential inherited by build'} }
  }
  if ($args | take 4) != [zigbuild --locked --release --target] { error make {msg: 'unexpected build command'} }
  let triple = $args | last
  log build $triple
  if ($env.FAKE_FAIL_BUILD? | default '') == $triple { exit 1 }
  let target = (open ($env.FAKE_PROJECT | path join release-targets.toml)).targets | where triple == $triple | first
  let version = (open ($env.FAKE_PROJECT | path join release.toml)).version
  let directory = $env.CARGO_TARGET_DIR | path join $triple release
  mkdir $directory
  let binary = $directory | path join $target.executable
  let description = match $triple {
    x86_64-unknown-linux-gnu => 'ELF 64-bit LSB x86-64 dynamically linked'
    aarch64-unknown-linux-gnu => 'ELF 64-bit LSB ARM aarch64 dynamically linked'
    x86_64-unknown-linux-musl => 'ELF 64-bit LSB x86-64 statically linked'
    aarch64-unknown-linux-musl => 'ELF 64-bit LSB ARM aarch64 statically linked'
    aarch64-apple-darwin => 'Mach-O 64-bit arm64 executable'
    x86_64-pc-windows-gnu => 'PE32+ executable MS Windows x86-64'
  }
  $"#!/usr/bin/env bash\n# format: ($description)\nprintf 'fjx ($version)\\n'\n" | save --force --raw $binary
  ^chmod 755 $binary
}

export def fake-file [args: list<string>] {
  log format-check
  open --raw ($args | last) | lines | where {|line| $line | str starts-with '# format: ' } | first | str replace '# format: ' '' | print
}

def argument [args: list<string>, flag: string] {
  let index = $args | enumerate | where item == $flag | first | get index
  $args | get ($index + 1)
}

export def fake-curl [args: list<string>] {
  let output = argument $args '--output'
  let method = argument $args '--request'
  let url = $args | last
  let record = $env.FAKE_STATE | path join release.json
  let assets = $env.FAKE_STATE | path join assets
  log $method $url
  if $url =~ '/releases/download/' {
    let name = $url | path basename
    log download $name
    if ($env.FAKE_CORRUPT_DOWNLOAD? | default '') == 'true' { 'corrupt' | save --force --raw $output } else { cp --force ($assets | path join $name) $output }
    print --no-newline 200
    return
  }
  if $method == GET {
    if ($record | path exists) { cp --force $record $output; print --no-newline 200 } else { print --no-newline 404 }
    return
  }
  if $method == POST and ($url | str ends-with '/releases') {
    let body = open --raw (argument $args '--data-binary' | str substring 1..) | from json
    $body | merge {id: 42, assets: []} | to json | save --force --raw $record
    cp --force $record $output
    print --no-newline 201
    return
  }
  if $method == POST {
    let name = $url | split row '?name=' | last
    if ($env.FAKE_FAIL_UPLOAD? | default '') == $name { print --no-newline 503; return }
    let release = open $record
    if $name in ($release.assets | get name) { error make {msg: 'existing asset overwritten'} }
    let source = argument $args '--form' | str replace 'attachment=@' ''
    cp $source ($assets | path join $name)
    let asset = {id: (($release.assets | length) + 1), name: $name, browser_download_url: $"https://forge.example/forgejo/dalton/fjx/releases/download/fjx%2Fv1.2.10/($name)"}
    $release | update assets ($release.assets | append $asset) | to json | save --force --raw $record
    print --no-newline 201
    return
  }
  if $method == PATCH {
    let body = open --raw (argument $args '--data-binary' | str substring 1..) | from json
    if $body != {draft: false} { error make {msg: 'unexpected publication request'} }
    open $record | update draft false | to json | save --force --raw $record
    cp --force $record $output
    print --no-newline (if ($env.FAKE_AMBIGUOUS_PUBLISH? | default '') == 'true' { 503 } else { 200 })
    return
  }
  error make {msg: 'unexpected HTTP call'}
}
