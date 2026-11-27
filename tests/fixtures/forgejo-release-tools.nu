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
    # Forgejo syncs pushed tags in a queue. The sync publishes any draft with
    # that tag name. In v0.4.0 and v0.5.0 it ran 2-3 s after the push, once the
    # draft existed. Model that: it runs on the first API call after a release
    # exists. FAKE_TAG_SYNC=never models a queue slower than the job.
    if ($env.FAKE_TAG_SYNC? | default queued) != never { touch ($env.FAKE_STATE | path join pending-tag-sync) }
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
  let script = $"#!/usr/bin/env bash\n# format: ($description)\nprintf 'fjx ($version)\\n'\n"
  if $target.os == windows {
    # Like the real linker: the PE timestamp is SOURCE_DATE_EPOCH, else now.
    # FAKE_UNPINNED_LINK models a linker that ignores SOURCE_DATE_EPOCH.
    let pinned = if ($env.FAKE_UNPINNED_LINK? | default '') == 'true' { null } else { $env.SOURCE_DATE_EPOCH? }
    let stamp = $pinned | default (date now | format date '%s') | into int
    let header = 0x[4D 5A] | bytes add --end (0..57 | each { 0x[00] } | bytes collect) | bytes add --end 0x[40 00 00 00 50 45 00 00 64 86 06 00]
    $header | bytes add --end ($stamp | into binary | bytes at 0..<4) | bytes add --end ($"\n($script)" | into binary) | save --force --raw $binary
  } else { $script | save --force --raw $binary }
  ^chmod 755 $binary
}

export def fake-file [args: list<string>] {
  log format-check
  ^grep --text '^# format: ' ($args | last) | lines | first | str replace '# format: ' '' | print
}

def argument [args: list<string>, flag: string] {
  let index = $args | enumerate | where item == $flag | first | get index
  $args | get ($index + 1)
}

# Log every draft-to-public change with the asset count at that moment.
def make-public [record: path] {
  let release = open $record
  if $release.draft { log public ($release.assets | length | into string) }
  $release | update draft false | to json | save --force --raw $record
}

def run-tag-sync [record: path] {
  let pending = $env.FAKE_STATE | path join pending-tag-sync
  if not ($pending | path exists) or not ($record | path exists) { return }
  rm $pending
  log tag-sync
  make-public $record
}

export def fake-curl [args: list<string>] {
  let output = argument $args '--output'
  let method = argument $args '--request'
  let url = $args | last
  let record = $env.FAKE_STATE | path join release.json
  let assets = $env.FAKE_STATE | path join assets
  run-tag-sync $record
  log $method $url
  # Like Forgejo 17 with the Actions job token: the web download routes give
  # it public read access only, so draft asset bytes are 404 there. The API
  # gives it write access but returns asset metadata, never bytes.
  if $url =~ '/releases/download/' or $url =~ '/attachments/' {
    let name = $url | path basename
    log download $name
    if (open $record).draft { print --no-newline 404; return }
    let file = $assets | path join $name
    if ($env.FAKE_CORRUPT_DOWNLOAD? | default '') == 'true' {
      # Same size, different bytes: only a byte comparison can catch it.
      open --raw $file | into binary | bytes reverse | save --force --raw $output
    } else { cp --force $file $output }
    print --no-newline 200
    return
  }
  if $method == GET {
    if not ($record | path exists) { print --no-newline 404; return }
    let release = open $record
    # FAKE_DRAFT_METADATA models a draft whose first asset record is wrong.
    let shown = if $release.draft and ($release.assets | is-not-empty) {
      match ($env.FAKE_DRAFT_METADATA? | default '') {
        size => ($release | update assets {|r| $r.assets | update 0 {|a| $a | update size ($a.size - 1) } })
        origin => ($release | update assets {|r| $r.assets | update 0 {|a| $a | update browser_download_url ($a.browser_download_url | str replace 'https://forge.example' 'https://evil.example') } })
        id => ($release | update assets {|r| $r.assets | update 0 {|a| $a | reject id } })
        _ => $release
      }
    } else { $release }
    $shown | to json | save --force --raw $output
    print --no-newline 200
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
    let size = ls ($assets | path join $name) | first | get size | into int
    let id = ($release.assets | length) + 1
    let asset = {id: $id, name: $name, size: $size, uuid: $"00000000-0000-0000-0000-($id | fill --width 12 --alignment right --character '0')", type: attachment, browser_download_url: $"https://forge.example/forgejo/dalton/fjx/releases/download/fjx%2Fv1.2.10/($name)"}
    $release | update assets ($release.assets | append $asset) | to json | save --force --raw $record
    print --no-newline 201
    return
  }
  if $method == PATCH {
    let body = open --raw (argument $args '--data-binary' | str substring 1..) | from json
    if $body != {draft: false} { error make {msg: 'unexpected publication request'} }
    make-public $record
    cp --force $record $output
    print --no-newline (if ($env.FAKE_AMBIGUOUS_PUBLISH? | default '') == 'true' { 503 } else { 200 })
    return
  }
  error make {msg: 'unexpected HTTP call'}
}
