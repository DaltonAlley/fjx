def argument [args: list<string>, name: string] {
  let index = $args | enumerate | where item == $name | first | get index
  $args | get ($index + 1)
}

def respond [args: list<string>, status: string, body: any] {
  $body | to json --raw | save --force --raw (argument $args '--output')
  print --no-newline $status
}

export def fake-git [args: list<string>] {
  let tag = $args | last | str replace '^{}' ''
  let hash = if ($env.FAKE_TAG_MISMATCH? | default '') == 'true' and ($args | str join ' ' | str contains github.com) { 'bbbb' } else { 'aaaa' }
  print $"($hash)\t($tag)"
}

export def fake-curl [args: list<string>] {
  let configuration = open --raw /dev/stdin
  let url = $args | last
  let method = argument $args '--request'
  let parsed = $url | url parse
  let github = $parsed.host in [api.github.com uploads.github.com]
  let cdn = $parsed.host == release-assets.githubusercontent.com
  let secret = if $cdn { '' } else if $github { 'github-fixture-secret' } else { 'forgejo-fixture-secret' }
  if ($args | str join ' ') =~ 'fixture-secret' { error make {msg: 'token in argv'} }
  if $cdn and $configuration != '' { error make {msg: 'credential forwarded to CDN'} }
  if not $cdn and not ($configuration | str contains $secret) { error make {msg: 'wrong origin credential'} }
  let root = $env.FAKE_MIRROR_STATE
  {method: $method, url: $url} | to json --raw | $in + (char nl) | save --append --raw ($root | path join calls)
  '' | save --force --raw (argument $args '--dump-header')
  let source = open ($root | path join source.json)
  let state = $root | path join github.json
  if $cdn {
    cp ($root | path join github-assets ($parsed.path | path basename)) (argument $args '--output')
    print --no-newline 200
    return
  }
  if not $github {
    if ($parsed.path | str contains '/releases/download/') {
      cp ($root | path join source-assets ($parsed.path | path basename)) (argument $args '--output')
      print --no-newline 200
    } else if ($parsed.path | str ends-with '/releases/47') {
      # Download counters change while copying; they are not content identity.
      respond $args 200 ($source | update assets {|r| $r.assets | each { insert download_count 99 } })
    } else {
      if ($env.FAKE_PAGINATE? | default '') == 'true' {
        if ($parsed.query | str contains 'page=1&') {
          respond $args 200 (1..100 | each {|id| $source | merge {id: $id, draft: true} })
        } else { respond $args 200 [$source] }
      } else { respond $args 200 [$source] }
    }
    return
  }
  if ($parsed.path | str ends-with '/releases') and $method == GET {
    respond $args 200 (if ($state | path exists) { [(open $state)] } else { [] })
    return
  }
  if $method == POST and $parsed.host == api.github.com {
    let body = open --raw ((argument $args '--data-binary') | str substring 1..) | from json
    let release = $body | merge {id: 88, assets: []}
    $release | to json | save --force $state
    respond $args 201 $release
    return
  }
  if $method == POST {
    let release = open $state
    let name = $parsed.query | str replace 'name=' ''
    let file = (argument $args '--data-binary') | str substring 1..
    cp $file ($root | path join github-assets $name)
    let asset = {id: (1000 + ($release.assets | length)), name: $name, size: (ls $file | first | get size | into int)}
    $release | update assets ($release.assets | append $asset) | to json | save --force $state
    if ($env.FAKE_UPLOAD_LOST? | default '') == 'true' { exit 7 }
    respond $args 201 $asset
    return
  }
  let release = open $state
  if ($parsed.path | str contains '/releases/assets/') {
    let id = $parsed.path | path basename | into int
    let asset = $release.assets | where id == $id | first
    let host = if ($env.FAKE_FOREIGN_CDN? | default '') == 'true' { 'evil.example' } else { 'release-assets.githubusercontent.com' }
    $"HTTP/1.1 302 Found\r\nLocation: https://($host)/($asset.name)\r\n\r\n" | save --force --raw (argument $args '--dump-header')
    respond $args 302 {}
  } else if ($parsed.path | str ends-with '/assets') {
    respond $args 200 $release.assets
  } else if $method == PATCH {
    let body = open --raw ((argument $args '--data-binary') | str substring 1..) | from json
    let updated = $release | merge $body
    $updated | to json | save --force $state
    respond $args 200 $updated
  } else { respond $args 200 $release }
}
