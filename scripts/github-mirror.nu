#!/usr/bin/env nu

# Repository automation, not a second release builder. Forgejo owns the bytes.
use package-release.nu [release-targets]

const source = 'https://dyad.tail77425.ts.net/forgejo'
const source_repo = 'dalton/fjx'
const destination = 'DaltonAlley/fjx'

def fail [message: string] { error make {msg: $"fjx GitHub mirror: ($message)"} }

# Tokens travel on stdin, never argv. Authenticated requests never redirect.
# Do not print curl errors or response bodies: either may contain credentials.
def request [method: string, url: string, directory: path, --body: record, --file: path, --binary, --anonymous] {
  let parsed = $url | url parse
  if $parsed.scheme != https or ($parsed.username? | default '') != '' or ($parsed.password? | default '') != '' {
    fail 'request must use HTTPS without user information'
  }
  let github = $parsed.host in [api.github.com uploads.github.com]
  let forgejo = ($url | str starts-with $"($source)/")
  if not $anonymous and not ($github or $forgejo) { fail 'refusing authentication outside the configured services' }
  let token = if $anonymous { '' } else if $github {
    $env.GH_RELEASE_MIRROR_TOKEN? | default ''
  } else { $env.FORGEJO_TOKEN? | default '' }
  if not $anonymous and ($token | str trim | is-empty) { fail 'required mirror credential is missing' }
  if $token =~ '[\r\n]' { fail 'invalid mirror credential' }
  let output = ^mktemp -p $directory response.XXXXXX | str trim
  let headers = ^mktemp -p $directory headers.XXXXXX | str trim
  let accept = if $binary { 'application/octet-stream' } else { 'application/json' }
  mut arguments = [--disable --silent --show-error --proto '=https' --max-redirs 0 --connect-timeout 20 --max-time 300 --config - --output $output --dump-header $headers --write-out '%{http_code}' --request $method --header $"Accept: ($accept)"]
  if $body != null {
    let data = ^mktemp -p $directory body.XXXXXX | str trim
    $body | to json --raw | save --force --raw $data
    $arguments = $arguments | append [--header 'Content-Type: application/json' --data-binary $"@($data)"]
  }
  if $file != null { $arguments = $arguments | append [--header 'Content-Type: application/octet-stream' --data-binary $"@($file)"] }
  let configuration = if $anonymous { '' } else {
    let scheme = if $github { 'Bearer' } else { 'token' }
    'header = ' + ($"Authorization: ($scheme) ($token)" | to json --raw) + (char nl)
  }
  let request_arguments = $arguments | append $url
  let result = do { $configuration | ^curl ...$request_arguments } | complete
  if $result.exit_code != 0 { fail 'HTTP transfer failed; rerun to reconcile remote state before retrying writes' }
  {status: ($result.stdout | str trim), output: $output, headers: $headers}
}

def api [service: string, method: string, path: string, directory: path, --body: record, --missing] {
  let base = if $service == forgejo { $"($source)/api/v1/repos/($source_repo)" } else { $"https://api.github.com/repos/($destination)" }
  let result = request $method $"($base)/($path)" $directory --body $body
  if $missing and $result.status == '404' { return null }
  if $result.status not-in ['200' '201' '204'] {
    fail $"($service) ($method) failed with HTTP ($result.status); inspect the run and rerun after resolving the failure"
  }
  if $result.status == '204' { return null }
  try { open --raw $result.output | from json } catch { fail $"($service) returned invalid JSON" }
}

def list-all [service: string, path: string, directory: path] {
  mut items = []
  for page in 1..100 {
    let batch = api $service GET $"($path)?per_page=100&page=($page)&limit=100" $directory
    if ($batch | describe) !~ '^list|^table' { fail 'expected a paginated array' }
    $items = $items | append $batch
    if ($batch | length) < 100 { return $items }
  }
  fail 'pagination exceeded 10,000 records; refusing an incomplete mirror'
}

export def release-identity [release: record] {
  if ($release.draft? | describe) != bool or ($release.prerelease? | describe) != bool {
    fail 'release must declare draft and prerelease state'
  }
  let tag = $release.tag_name? | default ''
  if $tag !~ '^fjx/v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z.-]+)?$' {
    fail 'release tag must use fjx/vVERSION'
  }
  let version = $tag | str replace 'fjx/v' ''
  if ($version | str contains '-') and not $release.prerelease { fail 'prerelease version must be marked as a prerelease' }
  if ($release.id? | describe) != int or $release.id <= 0 { fail 'release has no valid ID' }
  {tag_name: $tag, name: ($release.name? | default ''), body: ($release.body? | default ''), prerelease: $release.prerelease}
}

export def asset-names [tag: string] {
  let version = $tag | str replace 'fjx/v' ''
  release-targets | each {|target|
    let name = $"fjx-($version)-($target.triple).($target.archive)"
    [$name $"($name).sha256"]
  } | flatten | sort
}

export def validate-assets [assets: list, tag: string, --partial] {
  let names = $assets | each {|asset| $asset.name? | default '' }
  let expected = asset-names $tag
  if ($names | uniq | length) != ($names | length) or ($names | any {|name| $name not-in $expected }) {
    fail 'duplicate or unexpected release assets; refusing to overwrite or delete them'
  }
  if not $partial and ($names | sort) != $expected { fail 'published source release does not have all twelve assets' }
  for asset in $assets {
    if ($asset.id? | describe) != int or $asset.id <= 0 or ($asset.size? | describe) != int or $asset.size <= 0 {
      fail 'asset has an invalid ID or size'
    }
  }
}

def tag-refs [url: string, tag: string] {
  let result = do { ^git ls-remote --tags $url $"refs/tags/($tag)" $"refs/tags/($tag)^{}" } | complete
  if $result.exit_code != 0 { fail 'could not read remote release tag' }
  $result.stdout | lines | sort
}

def verify-tag [tag: string, --wait] {
  let expected = tag-refs $"($source)/($source_repo).git" $tag
  mut actual = tag-refs $"https://github.com/($destination).git" $tag
  # Right after a release, the native push mirror may not have sent the new
  # tag yet. Wait a bounded time for a missing tag (read-only). A different
  # tag hash still fails at once.
  if $wait and ($expected | is-not-empty) and ($actual | is-empty) {
    let limit = $env.FJX_MIRROR_TAG_WAIT_SECONDS? | default '300' | into int | into duration --unit sec
    let step = [$limit 10sec] | math min
    let deadline = (date now) + $limit
    while ($actual | is-empty) and (date now) < $deadline {
      sleep $step
      $actual = tag-refs $"https://github.com/($destination).git" $tag
    }
  }
  if ($expected | is-empty) or $actual != $expected {
    fail 'release tag hashes differ; rerun after the native code mirror succeeds (tags are never created or moved here)'
  }
}

def download-source [asset: record, tag: string, destination: path, directory: path] {
  let encoded = $tag | str replace --all '/' '%2F'
  let expected = $"($source)/($source_repo)/releases/download/($encoded)/($asset.name)"
  if ($asset.browser_download_url? | default '') != $expected { fail 'source asset URL must identify this exact repository, tag, and filename' }
  let response = request GET $expected $directory --binary
  if $response.status != '200' { fail 'could not download source asset without redirecting authentication' }
  mv $response.output $destination
}

def download-github [asset: record, target: path, directory: path] {
  let response = request GET $"https://api.github.com/repos/($destination)/releases/assets/($asset.id)" $directory --binary
  if $response.status == '200' { mv $response.output $target; return }
  if $response.status != '302' { fail 'could not download GitHub asset for verification' }
  let locations = open --raw $response.headers | lines | parse --regex '(?i)^location:\s*(?<url>[^\r]+)\r?$'
  if ($locations | length) != 1 { fail 'GitHub asset redirect has no unique location' }
  let url = $locations.0.url | str trim
  let parsed = $url | url parse
  if $parsed.scheme != https or $parsed.host not-in [release-assets.githubusercontent.com objects.githubusercontent.com] {
    fail 'GitHub asset redirect must use the GitHub release CDN'
  }
  # The signed CDN request receives no GitHub or Forgejo authorization header.
  let downloaded = request GET $url $directory --binary --anonymous
  if $downloaded.status != '200' { fail 'GitHub release CDN download failed' }
  mv $downloaded.output $target
}

def digest [file: path] { open --raw $file | hash sha256 }

def verify-checksums [directory: path, tag: string] {
  for name in (asset-names $tag | where {|name| not ($name | str ends-with '.sha256') }) {
    let checksum = open --raw ($directory | path join $"($name).sha256") | str trim
    if $checksum != $"(digest ($directory | path join $name))  ($name)" { fail 'source archive checksum does not match its checksum file' }
  }
}

def same-metadata [release: record, identity: record] {
  ($release | select tag_name name body prerelease | update body {|r| $r.body | default '' }) == $identity
}

def inventory [assets: list] {
  $assets | sort-by name | select id name size browser_download_url
}

def sync-one [source_release: record, directory: path] {
  let identity = release-identity $source_release
  validate-assets $source_release.assets $identity.tag_name
  verify-tag $identity.tag_name --wait
  let assets_directory = $directory | path join source-assets
  let downloads = $directory | path join github-assets
  mkdir $assets_directory $downloads
  for asset in $source_release.assets {
    let file = $assets_directory | path join $asset.name
    download-source $asset $identity.tag_name $file $directory
    if (ls $file | first | get size | into int) != $asset.size { fail 'source asset size changed during download' }
  }
  verify-checksums $assets_directory $identity.tag_name

  # Listing includes drafts; the tags endpoint alone can miss a partial draft.
  let candidates = list-all github releases $directory | where tag_name == $identity.tag_name
  if ($candidates | length) > 1 { fail 'multiple GitHub releases use the same tag' }
  let existing = $candidates | first 1
  let release = if ($existing | is-empty) {
    api github POST releases $directory --body ($identity | merge {draft: true, make_latest: 'false'})
  } else { $existing.0 }
  if ($release.id? | describe) != int or $release.id <= 0 or $release.tag_name != $identity.tag_name { fail 'invalid GitHub release identity' }
  let assets = list-all github $"releases/($release.id)/assets" $directory
  validate-assets $assets $identity.tag_name --partial
  if not $release.draft and ($assets | length) != 12 { fail 'existing public GitHub release is incomplete; explicit repair is required' }
  for source_asset in $source_release.assets {
    let matches = $assets | where name == $source_asset.name
    if ($matches | is-empty) {
      # Only missing names are uploaded. An uncertain response stops this run;
      # the next run lists and verifies remote bytes before another write.
      let uploaded = request POST $"https://uploads.github.com/repos/($destination)/releases/($release.id)/assets?name=($source_asset.name)" $directory --file ($assets_directory | path join $source_asset.name)
      if $uploaded.status != '201' { fail 'asset upload failed; rerun to reconcile the existing release' }
    }
  }
  let final_assets = list-all github $"releases/($release.id)/assets" $directory
  validate-assets $final_assets $identity.tag_name
  for asset in $final_assets {
    let downloaded = $downloads | path join $asset.name
    download-github $asset $downloaded $directory
    if (digest $downloaded) != (digest ($assets_directory | path join $asset.name)) {
      fail 'GitHub asset differs from Forgejo; refusing to replace it or publish the draft'
    }
  }
  # A source edit or withdrawal while uploading must not be published blindly.
  let current = api forgejo GET $"releases/($source_release.id)" $directory
  if $current.draft or not (same-metadata $current $identity) or (inventory $current.assets) != (inventory $source_release.assets) {
    fail 'source release changed while copying; rerun to reconcile it'
  }
  verify-tag $identity.tag_name
  if $release.draft or not (same-metadata $release $identity) {
    let latest = if $identity.prerelease { 'false' } else { 'legacy' }
    api github PATCH $"releases/($release.id)" $directory --body ($identity | merge {draft: false, make_latest: $latest}) | ignore
  }
  let final = api github GET $"releases/($release.id)" $directory
  if $final.draft or not (same-metadata $final $identity) { fail 'final release metadata differs from Forgejo' }
  print $"Verified ($identity.tag_name): notes, prerelease state, tag hashes, and twelve identical assets."
}

def main [] {
  let directory = mktemp -d
  try {
    let releases = list-all forgejo releases $directory
    for release in ($releases | where draft == false | sort-by id) {
      let stage = ^mktemp -d -p $directory release.XXXXXX | str trim
      sync-one $release $stage
    }
  } catch {|error|
    rm --recursive --force $directory
    error make $error.raw
  }
  rm --recursive --force $directory
}
