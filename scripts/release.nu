#!/usr/bin/env nu

const project_name = "fjx"
const script_dir = path self .

def fail [message: string] {
  error make {msg: $"fjx release: ($message)"}
}

def project-dir [] {
  let source_root = ($script_dir | path dirname | path dirname)
  let workspace_root = ($env.RELEASE_WORKSPACE_ROOT? | default $source_root)
  $workspace_root | path join $project_name
}

def valid-version [version: string] {
  $version =~ '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'
}

def version-greater [left: string, right: string] {
  let a = $left | split row . | each { into int }
  let b = $right | split row . | each { into int }
  if $a.0 != $b.0 { return ($a.0 > $b.0) }
  if $a.1 != $b.1 { return ($a.1 > $b.1) }
  $a.2 > $b.2
}

def checked-version [value: any, label: string] {
  if (($value | describe) != 'string') or not (valid-version $value) {
    fail $"invalid version in ($label)"
  }
  $value
}

def read-version-file [path: path, label: string] {
  if not ($path | path exists) { fail $"missing ($label)" }
  let value = try { (open --raw $path | from toml).version } catch { fail $"invalid version in ($label)" }
  checked-version $value $label
}

export def release-version [] {
  read-version-file ((project-dir) | path join release.toml) "fjx/release.toml"
}

def package-version [] {
  let path = ((project-dir) | path join Cargo.toml)
  if not ($path | path exists) { fail "missing fjx/Cargo.toml" }
  let value = try { (open --raw $path | from toml).package.version } catch { fail "invalid version in fjx/Cargo.toml" }
  checked-version $value "fjx/Cargo.toml"
}

def lock-version [] {
  let path = ((project-dir) | path join Cargo.lock)
  if not ($path | path exists) { fail "missing fjx/Cargo.lock" }
  let packages = try { (open --raw $path | from toml).package } catch { fail "invalid fjx version in fjx/Cargo.lock" }
  let matches = ($packages | where name == $project_name)
  if ($matches | length) != 1 { fail "invalid fjx version in fjx/Cargo.lock" }
  let value = ($matches | first | get version)
  if (($value | describe) != 'string') or not (valid-version $value) {
    fail "invalid fjx version in fjx/Cargo.lock"
  }
  $value
}

export def validate-versions [] {
  let expected = (release-version)
  let cargo = (package-version)
  let lock = (lock-version)
  if $cargo != $expected {
    fail $"Cargo.toml version ($cargo) does not match release.toml ($expected)"
  }
  if $lock != $expected {
    fail $"Cargo.lock version ($lock) does not match release.toml ($expected)"
  }
  $expected
}

def next-version [version: string, bump: string] {
  let parts = ($version | split row '.' | each {|part| $part | into int })
  let major = ($parts | get 0)
  let minor = ($parts | get 1)
  let patch = ($parts | get 2)
  match $bump {
    patch => $"($major).($minor).($patch + 1)",
    minor => $"($major).($minor + 1).0",
    major => $"($major + 1).0.0",
    _ => { fail "bump must be patch, minor, or major" }
  }
}

def rewrite-version [source: path, destination: path, version: string, lock: bool] {
  let input = (open --raw $source)
  let pattern = if $lock {
    '(?ms)(\[\[package\]\]\s+name = "fjx"\s+version = ")[^"]+(".*)'
  } else {
    '(?m)^(version\s*=\s*")[^"]+(".*)$'
  }
  let output = ($input | str replace --regex $pattern $'${1}($version)${2}')
  if $output == $input { fail $"could not update ($source)" }
  $output | save --raw $destination
}

def rollback [files: list<path>, transaction: path] {
  for item in ($files | enumerate) {
    ^cp -p ($transaction | path join $"original-($item.index)") $item.item
  }
  rm -rf $transaction
}

export def prepare-version [bump: string, --dry-run] {
  let current = (validate-versions)
  let proposed = (next-version $current $bump)
  let result = [
    $"project=($project_name)"
    $"current_version=($current)"
    $"proposed_version=($proposed)"
    $"tag=fjx/v($proposed)"
    $"dry_run=($dry_run)"
  ] | str join (char nl)
  if $dry_run { print $result; return }

  let project = (project-dir)
  let files = [
    ($project | path join release.toml)
    ($project | path join Cargo.toml)
    ($project | path join Cargo.lock)
  ]
  let transaction = (^mktemp -d ($project | path join '.release-prepare.XXXXXX') | str trim)
  try {
    for item in ($files | enumerate) {
      ^cp -p $item.item ($transaction | path join $"original-($item.index)")
    }
    for item in ($files | enumerate) {
      rewrite-version $item.item ($transaction | path join $"staged-($item.index)") $proposed ($item.index == 2)
    }
    for item in ($files | enumerate) {
      ^mv ($transaction | path join $"staged-($item.index)") $item.item
      if ($env.RELEASE_WORKSPACE_ROOT? | is-not-empty) and (($env.RELEASE_PREPARE_FAIL_AFTER_MOVE? | default '') == $"($item.index + 1)") {
        fail "injected failure after move"
      }
    }
    if (($env.RELEASE_PREPARE_FAIL_VALIDATION? | default '') == 'true') { fail "injected final validation failure" }
    validate-versions | ignore
  } catch {|error|
    rollback $files $transaction
    error make $error.raw
  }
  rm -rf $transaction
  print $result
}

export def publication-plan [expected_revision: string, actual_revision: string, tag_status: int, source_date_epoch: string] {
  if $actual_revision != $expected_revision { fail $"checked-out revision ($actual_revision) does not match ($expected_revision)" }
  let version = (validate-versions)
  let tag = $"fjx/v($version)"
  match $tag_status {
    0 => { {publish: false, tag: $tag, version: $version, source_date_epoch: $source_date_epoch} },
    2 => { {publish: true, tag: $tag, version: $version, source_date_epoch: $source_date_epoch} },
    _ => { fail $"could not determine whether ($tag) exists; git ls-remote exited ($tag_status)" }
  }
}

def auth-arguments [] {
  let token = $env.FORGEJO_TOKEN? | default ""
  if ($token | str trim) == "" { return [] }
  let scope = validated-git-origin
  [
    -c http.followRedirects=false
    -c $"http.($scope).followRedirects=false"
    -c $"http.($scope).extraHeader=Authorization: token ($token)"
  ]
}

def url-origin [value: string, label: string] {
  let parsed = try { $value | url parse } catch { fail $"($label) is not a valid URL" }
  let scheme = $parsed.scheme? | default "" | str downcase
  let host = $parsed.host? | default "" | str downcase
  if $scheme == "" or $host == "" { fail $"($label) is not an absolute URL" }
  if (($parsed.username? | default "") != "") or (($parsed.password? | default "") != "") {
    fail $"($label) must not contain user information"
  }
  let explicit_port = $parsed.port? | default ""
  let port = if ($explicit_port | into string) != "" {
    try { $explicit_port | into int } catch { fail $"($label) has an invalid port" }
  } else {
    match $scheme { "https" => 443, "http" => 80, _ => 0 }
  }
  {scheme: $scheme, host: $host, port: $port}
}

def validated-git-origin [] {
  let api = $env.FORGEJO_API_URL? | default "" | str trim
  let repository = $env.FORGEJO_REPOSITORY? | default "" | str trim
  if $api == "" or $repository !~ '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$' {
    fail "Forgejo API URL and repository are required before Git authentication"
  }
  let api_origin = url-origin $api "Forgejo API URL"
  if $api_origin.scheme != "https" { fail "Git origin must use the exact HTTPS Forgejo origin before authentication" }
  let authority = if $api_origin.port == 443 { $api_origin.host } else { $"($api_origin.host):($api_origin.port)" }
  let scope = $"https://($authority)/"
  let expected = $"($scope)($repository).git"
  let remote = do { ^git remote get-url origin } | complete
  if $remote.exit_code != 0 or ($remote.stdout | str trim) != $expected {
    fail "Git origin must be the exact HTTPS Forgejo repository before authentication"
  }
  $scope
}

def release-api [method: string, path: string, --attachment: path] {
  let api = $env.FORGEJO_API_URL? | default "" | str trim
  let repository = $env.FORGEJO_REPOSITORY? | default "" | str trim
  let token = $env.FORGEJO_TOKEN? | default "" | str trim
  if $api == "" or $repository == "" or $token == "" { fail "Forgejo API URL, repository, and token are required for publication state and repairs" }
  if (url-origin $api "Forgejo API URL").scheme != "https" {
    fail "Forgejo API URL must use HTTPS before authentication"
  }
  let response = mktemp
  mut arguments = [--silent --show-error --proto '=https' --max-redirs 0 --output $response --write-out '%{http_code}' --request $method --header $"Authorization: token ($token)" --header 'Accept: application/json']
  if $attachment != null { $arguments = $arguments | append [--form $"attachment=@($attachment)"] }
  let request_arguments = $arguments | append $"($api)/repos/($repository)/($path)"
  let request = do { ^curl ...$request_arguments } | complete
  if $request.exit_code != 0 { rm --force $response; fail $"Forgejo API request failed with exit code ($request.exit_code)" }
  {status: ($request.stdout | str trim), response: $response}
}

def download-release-asset [asset: record, name: string] {
  let api = $env.FORGEJO_API_URL? | default "" | str trim
  let repository = $env.FORGEJO_REPOSITORY? | default "" | str trim
  let token = $env.FORGEJO_TOKEN? | default "" | str trim
  if $api == "" or $repository == "" or $token == "" {
    fail "Forgejo API URL, repository, and token are required to verify release assets"
  }
  let url = $asset.browser_download_url? | default "" | str trim
  if $url == "" { fail $"release asset ($name) has no download URL" }
  let api_origin = url-origin $api "Forgejo API URL"
  let asset_origin = url-origin $url $"release asset ($name) download URL"
  if $asset_origin.scheme != "https" or $api_origin.scheme != "https" or $asset_origin != $api_origin {
    fail $"release asset ($name) download URL must use the exact HTTPS Forgejo API origin"
  }
  let asset_id = $asset.id? | default null
  if (($asset_id | describe) !~ '^int') or $asset_id <= 0 {
    fail $"release asset ($name) has no valid Forgejo asset ID"
  }

  let downloaded = mktemp
  let request_url = $"($api)/repos/($repository)/releases/assets/($asset_id)"
  let request = do {
    ^curl --silent --show-error --proto '=https' --max-redirs 0 --output $downloaded --write-out '%{http_code}' --request GET --header $"Authorization: token ($token)" --header 'Accept: application/octet-stream' $request_url
  } | complete
  if $request.exit_code != 0 or ($request.stdout | str trim) != "200" {
    rm --force $downloaded
    fail $"could not download release asset ($name) without leaving the Forgejo origin"
  }
  $downloaded
}

def inspect-remote-tags [tag: string, revision: string, version: string] {
  let prefix = 'refs/tags/fjx/v'
  let remote = do { ^git ...(auth-arguments) ls-remote --tags origin $"($prefix)*" } | complete
  if $remote.exit_code != 0 { fail $"could not list fjx release tags; git ls-remote exited ($remote.exit_code)" }
  let refs = $remote.stdout | lines | each {|line|
    let fields = $line | split row --regex '\s+' | where {|field| $field != "" }
    if ($fields | length) == 2 { {sha: $fields.0, ref: $fields.1} } else { null }
  } | compact
  let versions = $refs | each {|entry| $entry.ref | str replace --regex '\^\{\}$' '' } | uniq | where {|ref| $ref | str starts-with $prefix } | each {|ref| $ref | str replace $prefix '' } | where {|candidate| valid-version $candidate }
  for released in ($versions | where $it != $version) {
    if not (version-greater $version $released) { fail $"release version ($version) must be greater than prior released version ($released)" }
  }
  let tag_ref = $"refs/tags/($tag)"
  let peeled_ref = $"($tag_ref)^{}"
  let exact = $refs | where {|entry| $entry.ref == $tag_ref or $entry.ref == $peeled_ref }
  if ($exact | is-empty) { return true }
  let peeled = $exact | where ref == $peeled_ref
  if ($peeled | length) != 1 { fail $"($tag) exists but is not one annotated tag; refusing to replace it" }
  if $peeled.0.sha != $revision { fail $"($tag) targets ($peeled.0.sha) instead of ($revision); refusing to move it" }
  false
}

def release-targets [] {
  let path = ((project-dir) | path join release-targets.toml)
  let manifest = try { open --raw $path | from toml } catch { fail "invalid fjx/release-targets.toml" }
  if $manifest.schema != 1 or ($manifest.targets | length) != 6 {
    fail "fjx/release-targets.toml must declare schema 1 and exactly six targets"
  }
  $manifest.targets
}

def asset-names [version: string] {
  release-targets | each {|target|
    let archive = $"fjx-($version)-($target.triple).($target.archive)"
    [$archive $"($archive).sha256"]
  } | flatten | sort
}

def expected-release [version: string, tag: string] {
  {tag_name: $tag, name: ('fjx (' + $version + ')'), body: "", draft: false, prerelease: false}
}

def verify-downloaded-assets [directory: path, version: string] {
  let expected = asset-names $version
  let actual = glob ($directory | path join '*') --no-dir | each { path basename } | sort
  if $actual != $expected { fail "downloaded release asset set is not exact" }
  for target in (release-targets) {
    let archive = $"fjx-($version)-($target.triple).($target.archive)"
    let checked = do { cd $directory; ^sha256sum --check --strict $"($archive).sha256" } | complete
    if $checked.exit_code != 0 { fail $"downloaded checksum failed for ($archive)" }
    let archive_path = ($directory | path join $archive)
    let contents = if $target.archive == tar.gz {
      ^tar -tzf $archive_path | lines
    } else if $target.archive == zip {
      ^unzip -Z1 $archive_path | lines
    } else {
      fail $"unsupported release archive format ($target.archive)"
    }
    if $contents != [$target.executable] { fail $"downloaded archive contents are invalid for ($archive)" }
  }
}

def validate-release [release: record, version: string, tag: string] {
  let actual = {
    tag_name: ($release.tag_name? | default null)
    name: ($release.name? | default null)
    body: ($release.body? | default null)
    draft: ($release.draft? | default null)
    prerelease: ($release.prerelease? | default null)
  }
  if $actual != (expected-release $version $tag) {
    fail "Forgejo release metadata is not exact; refusing ambiguous repair"
  }
}

def verify-remote-assets [release: record, version: string] {
  let expected = asset-names $version
  let local = (project-dir) | path join publish-assets
  let local_names = glob ($local | path join '*') --no-dir | each { path basename } | sort
  if $local_names != $expected { fail "locally checked release assets are unavailable or not exact" }
  let downloaded = ^mktemp -d | str trim
  try {
    for asset in $release.assets {
      let name = $asset.name
      let destination = $downloaded | path join $name
      let fetched = download-release-asset $asset $name
      mv $fetched $destination
      let comparison = do { ^cmp --silent ($local | path join $name) $destination } | complete
      if $comparison.exit_code != 0 { rm --recursive --force $downloaded; fail $"remote release asset ($name) does not match the locally checked output" }
    }
    verify-downloaded-assets $downloaded $version
  } catch {|error|
    rm --recursive --force $downloaded
    error make $error.raw
  }
  rm --recursive --force $downloaded
}

def write-output [plan: record, output: path] {
  [
    $"publish=($plan.publish)"
    $"needs_tag=($plan.needs_tag? | default $plan.publish)"
    $"needs_release=($plan.needs_release? | default $plan.publish)"
    $"needs_assets=($plan.needs_assets? | default $plan.publish)"
    $"missing_assets=($plan.missing_assets? | default '')"
    $"release_id=($plan.release_id? | default '')"
    $"tag=($plan.tag)"
    $"version=($plan.version)"
    $"source_date_epoch=($plan.source_date_epoch)"
  ] | str join (char nl) | $in + (char nl) | save --append --raw $output
}

def "main version" [] { release-version }
def "main validate" [] { validate-versions }
def "main validate-tag" [tag: string] {
  let expected = $"fjx/v(validate-versions)"
  if $tag != $expected { fail $"tag must be ($expected)" }
}
def "main prepare" [bump: string, --dry-run] { prepare-version $bump --dry-run=$dry_run }
def "main publish-plan" [expected_revision: string, source_date_epoch: string, output: path] {
  let actual_revision = (^git rev-parse HEAD | str trim)
  if $actual_revision != $expected_revision { fail $"checked-out revision ($actual_revision) does not match ($expected_revision)" }
  let version = (validate-versions)
  let tag = $"fjx/v($version)"
  let needs_tag = inspect-remote-tags $tag $expected_revision $version
  let request = release-api GET $"releases/tags/($tag | str replace --all '/' '%2F')"
  mut release_id = ""
  mut needs_release = false
  mut missing_assets = []
  match $request.status {
    "200" => {
      if $needs_tag { rm --force $request.response; fail $"($tag) has a Forgejo release but no verifiable remote tag; refusing ambiguous repair" }
      let release = try { open --raw $request.response | from json } catch { rm --force $request.response; fail "Forgejo returned an invalid release record" }
      if ($release.tag_name? | default "") != $tag or (($release.id? | describe) !~ '^int') { rm --force $request.response; fail "Forgejo returned an invalid release identity" }
      validate-release $release $version $tag
      $release_id = $release.id | into string
      let names = $release.assets? | default [] | each {|asset| $asset.name? | default "" }
      let expected = asset-names $version
      if ($names | uniq | length) != ($names | length) or ($names | any {|name| $name not-in $expected }) { rm --force $request.response; fail "release has duplicate or unexpected assets; refusing ambiguous repair" }
      $missing_assets = $expected | where $it not-in $names
    }
    "404" => { $needs_release = true; $missing_assets = asset-names $version }
    _ => { rm --force $request.response; fail $"could not determine whether the Forgejo release exists (HTTP ($request.status))" }
  }
  rm --force $request.response
  # A complete named asset set still needs a fresh local build so verify-final
  # can compare the remote bytes with output that passed the native checks.
  let needs_assets = true
  let complete = not ($needs_tag or $needs_release or $needs_assets)
  let plan = {publish: (not $complete), needs_tag: $needs_tag, needs_release: $needs_release, needs_assets: $needs_assets, missing_assets: ($missing_assets | str join ','), release_id: $release_id, tag: $tag, version: $version, source_date_epoch: $source_date_epoch}
  write-output $plan $output
}

def "main upload-missing-assets" [release_id: int, names: string] {
  let expected = asset-names (validate-versions)
  for name in ($names | split row ',' | where $it != "") {
    if $name not-in $expected { fail $"refusing unexpected attachment ($name)" }
    let file = ((project-dir) | path join publish-assets $name)
    if not ($file | path exists) { fail $"missing prepared attachment ($name)" }
    let request = release-api POST $"releases/($release_id)/assets?name=($name)" --attachment $file
    if $request.status != "201" { rm --force $request.response; fail $"Forgejo refused missing ($name) attachment (HTTP ($request.status)); no existing asset was overwritten" }
    rm --force $request.response
  }
}

def "main push-tag" [tag: string, revision: string] {
  let tag_object = do { ^git cat-file -t $"refs/tags/($tag)" } | complete
  let peeled = do { ^git rev-parse $"refs/tags/($tag)^{}" } | complete
  if $tag_object.exit_code != 0 or ($tag_object.stdout | str trim) != "tag" or $peeled.exit_code != 0 or ($peeled.stdout | str trim) != $revision {
    fail "refusing to push a missing, lightweight, or wrongly targeted release tag"
  }
  if ($env.FORGEJO_TOKEN? | default "" | str trim) == "" { fail "Forgejo token is required to push a release tag" }
  let pushed = do { ^git ...(auth-arguments) push origin $"refs/tags/($tag)" } | complete
  if $pushed.exit_code != 0 { fail "authenticated release tag push failed" }
}

def "main verify-final" [version: string, revision: string, tag: string] {
  if $version != (validate-versions) or $tag != $"fjx/v($version)" { fail "final release identity does not match project version" }
  if (inspect-remote-tags $tag $revision $version) { fail "final release tag is missing" }
  let request = release-api GET $"releases/tags/($tag | str replace --all '/' '%2F')"
  if $request.status != "200" { rm --force $request.response; fail "final Forgejo release is missing" }
  let release = try { open --raw $request.response | from json } catch { rm --force $request.response; fail "final Forgejo release is invalid JSON" }
  rm --force $request.response
  let names = $release.assets? | default [] | each {|asset| $asset.name? | default "" } | sort
  validate-release $release $version $tag
  if $names != (asset-names $version) { fail "final Forgejo release identity or assets are not exact" }
  verify-remote-assets $release $version
}

def main [] {
  fail "usage: fjx/scripts/release.nu (version|validate|validate-tag TAG|prepare [--dry-run] BUMP|publish-plan REVISION EPOCH OUTPUT|push-tag TAG REVISION|upload-missing-assets RELEASE_ID NAMES|verify-final VERSION REVISION TAG)"
}
