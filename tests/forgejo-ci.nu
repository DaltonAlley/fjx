#!/usr/bin/env nu

use ../scripts/forgejo-ci.nu [release-candidate]

const scripts = path self ../scripts
const targets = path self ../release-targets.toml
const tools = path self fixtures/forgejo-release-tools.nu

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

def test-candidates [] {
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
    cd ($root | path dirname)
    rm --recursive --force $root
    error make $error.raw
  }
  rm --recursive --force $root
}

def calls [state: path] {
  open --raw ($state | path join calls) | lines | each { from json }
}

def run-ci [project: path, arguments: list<string>, extra: record = {}] {
  with-env $extra {
    do { ^$nu.current-exe --no-config-file ($project | path join scripts forgejo-ci.nu) ...$arguments } | complete
  }
}

def fresh-checkout [project: path] {
  # A rerun gets a clean job container while the simulated server keeps state.
  rm --recursive --force ($project | path join publish-assets)
}

def test-orchestration [] {
  let root = mktemp -d
  let project = $root | path join fjx
  let state = $root | path join server
  let bin = $root | path join bin
  mkdir ($project | path join scripts) ($state | path join assets) $bin
  cp ($scripts | path join forgejo-ci.nu) ($scripts | path join release.nu) ($scripts | path join package-release.nu) ($scripts | path join release-smoke.nu) ($project | path join scripts)
  cp $targets $project
  cp $tools ($bin | path join tools.nu)
  for name in [git cargo curl file] {
    [
      '#!/usr/bin/env -S nu --no-config-file'
      $"use tools.nu [fake-($name)]"
      $"def --wrapped main [...args: string] { fake-($name) $args }"
    ] | str join (char nl) | save --raw ($bin | path join $name)
    ^chmod 755 ($bin | path join $name)
  }
  '' | save --raw ($state | path join calls)
  let real_git = which --all git | where type == external | first | get path
  try {
    cd $project
    git init | ignore
    let before = commit-version 1.2.9
    let unchanged = commit-version 1.2.9
    let revision = commit-version 1.2.10
    with-env {
      PATH: ($env.PATH | prepend $bin)
      FAKE_STATE: $state
      FAKE_PROJECT: $project
      FAKE_REAL_GIT: $real_git
      CARGO_TARGET_DIR: ($root | path join target)
      FORGEJO_API_URL: 'https://forge.example/forgejo/api/v1'
      FORGEJO_REPOSITORY: 'dalton/fjx'
      FORGEJO_TOKEN: fixture
      FORGEJO_EVENT_NAME: push
      FORGEJO_REF: refs/heads/main
      BEFORE_SHA: $before
    } {
      let output = $root | path join output
      let candidate = run-ci $project [candidate $before $revision $output]
      check ($candidate.exit_code == 0) $candidate.stderr
      check ((open --raw $output) == "release=true\n") 'main version increase must reach the release job'
      for context in [
        {FORGEJO_EVENT_NAME: pull_request}
        {FORGEJO_EVENT_NAME: workflow_dispatch}
        {FORGEJO_REF: refs/heads/feature}
      ] {
        rm $output
        let candidate = run-ci $project [candidate $before $revision $output] $context
        check ($candidate.exit_code == 0 and (open --raw $output) == "release=false\n") 'PRs, dispatches and other branches must not select publication'
        check ((run-ci $project [publish $revision] $context).exit_code != 0) 'direct publisher invocation must reject non-main-push contexts'
      }
      git checkout --detach $unchanged | ignore
      rm $output
      let candidate = run-ci $project [candidate $before $unchanged $output]
      check ($candidate.exit_code == 0 and (open --raw $output) == "release=false\n") 'unchanged versions must skip publication'
      check ((run-ci $project [publish $unchanged]).exit_code != 0) 'publisher must independently reject unchanged versions'
      check ((calls $state | is-empty)) 'non-candidates must not build or contact the release server'
      git checkout --detach $revision | ignore

      let failed = run-ci $project [publish $revision] {FAKE_FAIL_BUILD: x86_64-pc-windows-gnu}
      check ($failed.exit_code != 0) 'failed build must stop publication'
      check (not (($state | path join release.json) | path exists)) 'failed build created a release'
      check (not (($state | path join tag) | path exists)) 'failed build pushed a tag'
      check (calls $state | all {|call| $call.event not-in [POST PATCH push-tag DELETE] }) 'failed build attempted a write'

      # A Windows build whose PE timestamp is not the commit time is not
      # reproducible, so a rerun could not rebuild the same bytes.
      '' | save --force --raw ($state | path join calls)
      let unpinned = run-ci $project [publish $revision] {FAKE_UNPINNED_LINK: 'true'}
      check ($unpinned.exit_code != 0) 'unpinned Windows build must stop publication'
      check ($unpinned.stderr | str contains 'PE timestamp') $"unpinned build failure omitted its reason: ($unpinned.stderr)"
      check (calls $state | all {|call| $call.event not-in [POST PATCH push-tag DELETE] }) 'unpinned build attempted a write'
      fresh-checkout $project

      '' | save --force --raw ($state | path join calls)
      let first_asset = 'fjx-1.2.10-aarch64-apple-darwin.tar.gz'
      let failed_asset = $first_asset + '.sha256'
      let failed = run-ci $project [publish $revision] {FAKE_FAIL_UPLOAD: $failed_asset}
      check ($failed.exit_code != 0) 'failed upload must stop publication'
      let draft = open ($state | path join release.json)
      check ($draft.draft and ($draft.assets | get name) == [$first_asset]) 'failed upload must leave only a partial draft'
      let upload_calls = calls $state
      let first_write = $upload_calls | enumerate | where {|row| $row.item.event in [POST push-tag] } | first | get index
      let preparation = $upload_calls | take $first_write
      check (($preparation | where event == build | length) == 6) 'writes began before all targets built'
      check (($preparation | where event == format-check | length) == 6) 'writes began before local archive verification'
      check (($upload_calls | where event == PATCH | is-empty)) 'failed upload published a partial release'
      # Forgejo publishes a draft when it syncs a pushed tag of the same name.
      check (($upload_calls | where event == push-tag | is-empty)) 'tag was pushed before the draft was complete and verified'
      check (($upload_calls | where event == POST | where {|call| $call.value | str ends-with $failed_asset } | length) == 1) 'failed upload was retried blindly'

      # The job token cannot read draft bytes, so the draft is checked by its
      # API metadata. A wrong size, foreign URL or missing ID must stop before
      # the tag push and the public transition. Each rerun first replaces the
      # assets an earlier run left on the untagged draft. An asset without an
      # ID cannot be deleted, so that run stops before any write.
      for case in [[metadata deletes uploads]; [size 1 12] [origin 12 12] [id 0 0]] {
        fresh-checkout $project
        '' | save --force --raw ($state | path join calls)
        let bad = run-ci $project [publish $revision] {FAKE_DRAFT_METADATA: $case.metadata}
        check ($bad.exit_code != 0) $"wrong draft ($case.metadata) must stop publication"
        check ((open ($state | path join release.json)).draft) $"wrong draft ($case.metadata) published a release"
        let bad_calls = calls $state
        check (($bad_calls | where event == PATCH | is-empty)) 'wrong draft metadata reached the public transition'
        check (($bad_calls | where event == push-tag | is-empty)) 'wrong draft metadata reached the tag push that publishes a draft'
        check (($bad_calls | where event == download | is-empty)) 'draft check tried to download draft bytes'
        let writes = $bad_calls | where event in [DELETE POST] | get event
        check ($writes == (($case.deletes | if $in > 0 { 1..$in | each { 'DELETE' } } else { [] }) ++ ($case.uploads | if $in > 0 { 1..$in | each { 'POST' } } else { [] }))) $"rerun ($case.metadata) must delete every old draft asset, then upload all twelve: ($writes)"
      }

      fresh-checkout $project
      '' | save --force --raw ($state | path join calls)
      let ambiguous = run-ci $project [publish $revision] {FAKE_AMBIGUOUS_PUBLISH: 'true'}
      check ($ambiguous.exit_code != 0) 'ambiguous publication must stop and report failure'
      let attempts = calls $state
      check (($attempts | where event == POST | where {|call| $call.value | str ends-with '/releases' } | is-empty)) 'draft retry must not create a second release'
      check (($attempts | where event == DELETE | length) == 12 and ($attempts | where event == POST | length) == 12) 'untagged draft retry must replace all twelve assets'
      check (($attempts | where event == PATCH | length) == 1) 'publication write was retried'
      let publication = $attempts | enumerate | where {|row| $row.item.event == PATCH } | first | get index
      check (($attempts | take $publication | where event == download | is-empty)) 'publication tried to download draft bytes'
      check (($attempts | where event == push-tag | length) == 1) 'complete draft must push the tag once'
      check (($attempts | where event == public | get value) == ['12']) 'release must become public once, with all twelve assets'

      fresh-checkout $project
      '' | save --force --raw ($state | path join calls)
      let retry = run-ci $project [publish $revision]
      check ($retry.exit_code == 0) $retry.stderr
      let published = open ($state | path join release.json)
      check (not $published.draft and ($published.assets | length) == 12) 'recovery must verify a complete public release'
      check (calls $state | all {|call| $call.event not-in [POST PATCH push-tag DELETE] }) 'published retry must be read-only after the rebuild'
      check ((calls $state | where event == download | length) == 12) 'published retry must verify every attachment again'

      # An old public-but-partial release is an explicit repair, never appended.
      $published | update assets ($published.assets | drop 1) | to json | save --force --raw ($state | path join release.json)
      fresh-checkout $project
      '' | save --force --raw ($state | path join calls)
      check ((run-ci $project [publish $revision]).exit_code != 0) 'partial public release must be rejected'
      check (calls $state | all {|call| $call.event not-in [POST PATCH push-tag DELETE] }) 'partial public release was modified'

      # Also exercise a normal, unambiguous publication from an empty server,
      # with Forgejo's tag sync publishing the draft (queued) or not (never).
      # Before the fix, the queued sync published the draft while assets were
      # still uploading and publish-release failed with 'metadata is not exact'.
      for sync in [queued never] {
        rm --force ($state | path join release.json) ($state | path join tag) ($state | path join pending-tag-sync)
        rm --recursive ($state | path join assets)
        mkdir ($state | path join assets)
        do { ^$real_git tag --delete fjx/v1.2.10 } | complete | ignore
        fresh-checkout $project
        '' | save --force --raw ($state | path join calls)
        let success = run-ci $project [publish $revision] {FAKE_TAG_SYNC: $sync}
        check ($success.exit_code == 0) $"($sync) tag sync: ($success.stderr)"
        let events = calls $state
        check (not (open ($state | path join release.json)).draft) 'successful run left a draft'
        check (($events | where event == POST | length) == 13) 'new release must create one draft and twelve attachments'
        check (($events | where event == PATCH | length) == 1) 'successful run must publish once'
        check (($events | where event == download | length) == 12) 'successful run must verify all bytes once after publication'
        check (($events | where event == public | get value) == ['12']) $"($sync) tag sync: release must become public once, with all twelve assets"
        check (($events | where event == tag-sync | length) == (if $sync == queued { 1 } else { 0 })) 'tag sync model did not run as configured'
        let tag_push = $events | enumerate | where {|row| $row.item.event == push-tag } | first | get index
        check (($events | take $tag_push | where event == download | is-empty)) 'tag push must not wait on draft bytes the job token cannot read'
        check (($events | skip $tag_push | where event == download | length) == 12) 'public release bytes must be verified after the tag push'
      }

      # Issue #27: an untagged draft holds a same-size Windows zip from an
      # earlier run. The rerun must replace it, not publish the old bytes.
      rm --force ($state | path join release.json) ($state | path join tag) ($state | path join pending-tag-sync)
      rm --recursive ($state | path join assets)
      mkdir ($state | path join assets)
      do { ^$real_git tag --delete fjx/v1.2.10 } | complete | ignore
      fresh-checkout $project
      check ((run-ci $project [publish $revision] {FAKE_DRAFT_METADATA: size}).exit_code != 0) 'stale draft setup must stop before the tag push'
      let windows_zip = $state | path join assets 'fjx-1.2.10-x86_64-pc-windows-gnu.zip'
      open --raw $windows_zip | into binary | bytes reverse | save --force --raw $windows_zip
      fresh-checkout $project
      '' | save --force --raw ($state | path join calls)
      let replaced = run-ci $project [publish $revision]
      check ($replaced.exit_code == 0) $"stale draft assets must be replaced: ($replaced.stderr)"
      let events = calls $state
      check (($events | where event == DELETE | length) == 12) 'rerun must delete every stale draft asset'
      check (($events | where event == POST | length) == 12) 'rerun must upload all twelve assets again'
      let last_delete = $events | enumerate | where {|row| $row.item.event == DELETE } | last | get index
      let tag_push = $events | enumerate | where {|row| $row.item.event == push-tag } | first | get index
      check ($last_delete < $tag_push) 'draft assets must be deleted before the tag push'

      # Once the tag is pushed, Forgejo may publish the draft at any moment.
      # A rerun must keep the draft assets and only finish publication.
      open ($state | path join release.json) | update draft true | to json | save --force --raw ($state | path join release.json)
      fresh-checkout $project
      '' | save --force --raw ($state | path join calls)
      let tagged = run-ci $project [publish $revision]
      check ($tagged.exit_code == 0) $"tagged draft rerun failed: ($tagged.stderr)"
      check (calls $state | all {|call| $call.event not-in [POST DELETE push-tag] }) 'tagged draft assets were deleted or uploaded'
      check ((calls $state | where event == PATCH | length) == 1) 'tagged draft must be published once'
      # The delete helper checks the tag itself, right before it deletes.
      open ($state | path join release.json) | update draft true | to json | save --force --raw ($state | path join release.json)
      '' | save --force --raw ($state | path join calls)
      let guarded = do { ^$nu.current-exe --no-config-file ($project | path join scripts release.nu) delete-draft-assets 42 $revision } | complete
      check ($guarded.exit_code != 0 and ($guarded.stderr | str contains 'already pushed')) $"delete helper ignored the pushed tag: ($guarded.stderr)"
      check (calls $state | all {|call| $call.event != DELETE }) 'delete helper deleted assets after the tag push'
      open ($state | path join release.json) | update draft false | to json | save --force --raw ($state | path join release.json)

      # Known limit: a same-size byte mismatch is only caught after publication.
      # The job must still fail, and a rerun must not change the public release.
      rm --force ($state | path join release.json) ($state | path join tag) ($state | path join pending-tag-sync)
      rm --recursive ($state | path join assets)
      mkdir ($state | path join assets)
      do { ^$real_git tag --delete fjx/v1.2.10 } | complete | ignore
      fresh-checkout $project
      '' | save --force --raw ($state | path join calls)
      let late = run-ci $project [publish $revision] {FAKE_CORRUPT_DOWNLOAD: 'true'}
      check ($late.exit_code != 0) 'same-size byte mismatch must fail the job'
      check ($late.stderr | str contains 'does not match the locally checked output') $"same-size mismatch must be reported: ($late.stderr)"
      check (not (open ($state | path join release.json)).draft) 'same-size mismatch is expected to be found only after publication'
      fresh-checkout $project
      '' | save --force --raw ($state | path join calls)
      check ((run-ci $project [publish $revision] {FAKE_CORRUPT_DOWNLOAD: 'true'}).exit_code != 0) 'rerun must still report the byte mismatch'
      check (calls $state | all {|call| $call.event not-in [POST PATCH push-tag DELETE] }) 'rerun modified a public release with mismatched bytes'
    }
  } catch {|error|
    cd ($root | path dirname)
    rm --recursive --force $root
    error make $error.raw
  }
  cd ($root | path dirname)
  rm --recursive --force $root
}

def main [] {
  test-candidates
  test-orchestration
  print 'Forgejo CI tests passed'
}
