#!/usr/bin/env nu

use ../scripts/github-mirror.nu [asset-names]
const script = path self ../scripts/github-mirror.nu
const tools = path self fixtures/github-mirror-tools.nu

def check [condition: bool, message: string] {
  if not $condition { error make {msg: $"GitHub mirror test: ($message)"} }
}

def calls [root: path] { open --raw ($root | path join calls) | lines | each { from json } }
def writes [root: path] { calls $root | where method != GET }
def run [extra: record = {}] {
  with-env $extra { do { ^$nu.current-exe --no-config-file $script } | complete }
}

def main [] {
  let root = mktemp -d
  let bin = $root | path join bin
  let assets = $root | path join source-assets
  let state = $root | path join github.json
  mkdir $bin $assets ($root | path join github-assets)
  '' | save ($root | path join calls)
  cp $tools ($bin | path join tools.nu)
  for name in [curl git] {
    [
      '#!/usr/bin/env -S nu --no-config-file'
      $"use tools.nu [fake-($name)]"
      $"def --wrapped main [...args: string] { let input = $in; try { $input | fake-($name) $args } catch {|e| $e.rendered | save --force --raw \($env.FAKE_MIRROR_STATE | path join mock-error\); error make $e.raw } }"
    ] | str join (char nl) | save ($bin | path join $name)
    ^chmod 755 ($bin | path join $name)
  }
  for name in (asset-names fjx/v1.2.3 | where {|n| not ($n | str ends-with '.sha256') }) {
    $"fixture bytes for ($name)" | save --raw ($assets | path join $name)
    $"(open --raw ($assets | path join $name) | hash sha256)  ($name)\n" | save --raw ($assets | path join $"($name).sha256")
  }
  let release = {
    id: 47, tag_name: 'fjx/v1.2.3', name: 'A release', body: "Notes\nwith newlines", draft: false, prerelease: false
    assets: (asset-names fjx/v1.2.3 | enumerate | each {|entry|
      {id: ($entry.index + 1), name: $entry.item, size: (ls ($assets | path join $entry.item) | first | get size | into int), browser_download_url: $"https://dyad.tail77425.ts.net/forgejo/dalton/fjx/releases/download/fjx%2Fv1.2.3/($entry.item)"}
    })
  }
  let source = $root | path join source.json
  $release | to json | save $source
  try {
    with-env {
      PATH: ($env.PATH | prepend $bin)
      FAKE_MIRROR_STATE: $root
      GH_RELEASE_MIRROR_TOKEN: github-fixture-secret
      FORGEJO_TOKEN: forgejo-fixture-secret
      FJX_MIRROR_TAG_WAIT_SECONDS: '2'
    } {
      $release | update draft true | to json | save --force $source
      let skipped = run
      check ($skipped.exit_code == 0) $skipped.stderr
      check ($skipped.exit_code == 0 and (writes $root | is-empty)) 'source drafts must never be published'
      $release | update assets ($release.assets | skip 1) | to json | save --force $source
      check ((run).exit_code != 0 and (writes $root | is-empty)) 'incomplete source release must fail before writes'
      $release | to json | save --force $source
      check ((run {FAKE_TAG_MISMATCH: 'true'}).exit_code != 0 and (writes $root | is-empty)) 'mismatched tags must stop copying'
      let started = date now
      check ((run {FAKE_TAG_LATE: '1000'}).exit_code != 0 and (writes $root | is-empty)) 'a tag that never reaches GitHub must fail before writes'
      check (((date now) - $started) >= 2sec) 'a missing GitHub tag must be waited for before failing'
      rm --force ($root | path join tag-lookups)
      let bad = $release | update assets {|r| $r.assets | update 0 {|a| $a | update browser_download_url 'https://evil.example/asset' } }
      $bad | to json | save --force $source
      check ((run).exit_code != 0 and (writes $root | is-empty)) 'foreign source asset URL must fail before writes'
      $release | to json | save --force $source
      let checksum = $assets | path join ($release.assets.1.name)
      let original = open --raw $checksum
      'bad checksum' | save --force --raw $checksum
      check ((run).exit_code != 0 and (writes $root | is-empty)) 'bad checksums must fail before writes'
      $original | save --force --raw $checksum

      let interrupted = run {FAKE_UPLOAD_LOST: 'true'}
      check ($interrupted.exit_code != 0) 'lost upload response must stop without retrying'
      check ((open $state).draft and ((open $state).assets | length) == 1) 'lost response must preserve the partial draft'
      check ((writes $root | length) == 2) 'ambiguous upload must not be retried in the same run'
      '' | save --force ($root | path join calls)
      # The release job copies right after publishing; the tag may arrive late.
      let recovered = run {FAKE_PAGINATE: 'true', FAKE_TAG_LATE: '1'}
      check ($recovered.exit_code == 0) $recovered.stderr
      check ((open --raw ($root | path join tag-lookups) | into int) >= 2) 'late GitHub tag was not looked up again'
      let published = open $state
      check (not $published.draft and ($published.assets | length) == 12 and $published.body == $release.body) 'rerun must finish and publish the same release'
      check ((writes $root | where method == POST | length) == 11) 'rerun uploaded an existing asset again'
      '' | save --force ($root | path join calls)
      let repeated = run
      check ($repeated.exit_code == 0 and (writes $root | is-empty)) 'complete rerun must verify bytes without writes'

      $release | merge {body: 'Updated notes', prerelease: true} | to json | save --force $source
      let edited = run
      check ($edited.exit_code == 0) $edited.stderr
      check ((open $state).prerelease and (open $state).make_latest == 'false' and (open $state).body == 'Updated notes') 'prerelease edit must preserve state and never become stable/latest'
      '' | save --force ($root | path join calls)
      let redirect = run {FAKE_FOREIGN_CDN: 'true'}
      check ($redirect.exit_code != 0 and (writes $root | is-empty)) 'foreign CDN must fail without writes'
      check (calls $root | all {|call| not ($call.url | str contains evil.example) }) 'foreign CDN was contacted'
      'corruption' | save --force --raw ($root | path join github-assets $release.assets.0.name)
      check ((run).exit_code != 0 and (writes $root | is-empty)) 'conflicting bytes must fail without overwrite'
      cp ($assets | path join $release.assets.0.name) ($root | path join github-assets $release.assets.0.name)
      open $state | update assets {|r| $r.assets | skip 1 } | to json | save --force $state
      check ((run).exit_code != 0 and (writes $root | is-empty)) 'partial public release needs explicit repair'
      check (not ((open --raw ($root | path join calls)) =~ 'fixture-secret')) 'credentials appeared in request logs'
      for result in [$interrupted $recovered $repeated $edited $redirect] {
        check (not (($result.stdout + $result.stderr) =~ 'fixture-secret')) 'credentials appeared in output'
      }
    }
  } catch {|error|
    if (($root | path join mock-error) | path exists) { print --stderr (open --raw ($root | path join mock-error)) }
    rm --recursive --force $root
    error make $error.raw
  }
  rm --recursive --force $root
  print 'GitHub mirror tests passed'
}
