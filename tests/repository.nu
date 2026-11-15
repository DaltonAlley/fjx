#!/usr/bin/env nu

const project_dir = path self ..

def check [condition: bool, message: string] {
  if not $condition { error make {msg: $"repository test: ($message)"} }
}

def test-package-contract [] {
  let manifest = open Cargo.toml
  check ($manifest.package.publish == false) 'package publication must remain disabled'
  check (not ('src/lib.rs' | path exists)) 'fjx must remain a single binary without a library API'
  check ($manifest.package.license == 'MIT') 'license metadata must match LICENSE'
  check ((open --raw LICENSE) | str starts-with 'MIT License') 'MIT license text is missing'
  check ($manifest.package.readme | path exists) 'package readme does not exist'
  check (($manifest.dependencies | columns | sort) == [lexopt rpassword serde serde_json ureq]) 'runtime dependency allowlist changed'
  check ($manifest.dependencies.ureq.default-features == false) 'ureq default features must stay disabled'
  check ($manifest.dependencies.ureq.features == [tls]) 'ureq must use the Rustls TLS feature'
  for dependency in ($manifest.dependencies | values) {
    let requirement = if ($dependency | describe) == 'string' { $dependency } else { $dependency.version }
    check ($requirement | str starts-with '=') 'runtime dependency versions must be pinned exactly'
  }
}

def test-ci-contract [] {
  let workflow = open .github/workflows/ci.yml
  let events = $workflow.on | columns
  check ('pull_request' in $events) 'pull requests must run CI'
  check ('push' in $events) 'pushes must run CI'
  check ('main' in $workflow.on.push.branches) 'main must run CI'
  check (not ('pull_request_target' in $events)) 'do not run untrusted changes in a privileged PR workflow'
  check (not ('workflow_run' in $events)) 'CI must not use a privileged follow-up trigger'
  check ($workflow.permissions == {contents: read}) 'CI must have read-only contents permissions'
  check (not ((open --raw .github/workflows/ci.yml) | str contains 'secrets.')) 'CI must not consume repository secrets'

  mut check_steps = 0
  for job in ($workflow.jobs | values) {
    check ($job.timeout-minutes > 0 and $job.timeout-minutes <= 30) 'jobs need a bounded timeout'
    check (($job.permissions? | default {contents: read}) == {contents: read}) 'job permissions must not elevate privileges'
    for step in $job.steps {
      let action = $step.uses? | default ''
      if ($action | is-not-empty) {
        check ($action =~ '^[A-Za-z0-9_.-]+/[A-Za-z0-9_./-]+@[0-9a-f]{40}$') 'actions must be pinned to a full commit SHA'
        if ($action | str starts-with 'actions/checkout@') {
          check ($step.with.persist-credentials == false) 'checkout must not leave credentials for PR code'
        }
      }
      if (($step.run? | default '' | str trim) == 'nu --no-config-file scripts/check.nu') {
        $check_steps = $check_steps + 1
      }
    }
  }
  check ($check_steps > 0) 'CI must run the same check command as contributors'

  let updates = (open .github/dependabot.yml).updates
  check (($updates | get package-ecosystem | sort) == [cargo github-actions]) 'monitor Cargo and Actions dependencies'
  for update in $updates {
    check ($update.directory == '/') 'Dependabot must target the standalone repository root'
  }
}

def test-documentation-links [] {
  let files = glob '*.md' | append (glob 'docs/**/*.md') | append (glob '.github/*.md')
  for file in $files {
    # Check relative inline Markdown file links, not external URLs or heading anchors.
    let links = open --raw $file | parse --regex '\]\((?<target>[^\s)]+)\)'
    for link in $links {
      if ($link.target =~ '^[A-Za-z][A-Za-z0-9+.-]*:') or ($link.target | str starts-with '#') {
        continue
      }
      let relative = $link.target | split row '#' | first
      let target = $file | path dirname | path join $relative
      check ($target | path exists) $"broken file link in ($file): ($link.target)"
    }
  }
}

def test-forgejo-ci-contract [] {
  let workflow = open .forgejo/workflows/ci.yaml
  let events = $workflow.on | columns
  check ('pull_request' in $events and 'push' in $events) 'Forgejo must check PRs and main'
  check (not ('pull_request_target' in $events)) 'Forgejo must not use privileged PR execution'
  check ($workflow.on.push.branches == [main]) 'Forgejo push checks must target main'
  let checks = $workflow.jobs.checks
  check ($checks.timeout-minutes == 5) 'PR checks must have a five-minute budget'
  check ($checks.runs-on == fjx-ci) 'checks must use the prepared runner'
  check ($checks.steps | any {|step| ($step.run? | default '') == 'nu --no-config-file scripts/check.nu' }) 'Forgejo must run the local build and E2E entry point'
  let release = $workflow.jobs.release
  check ($release.needs == checks) 'release publication must wait for checks'
  check ($release.runs-on != $checks.runs-on) 'release builds must have a separate runner slot'
  for condition in ["forgejo.event_name == 'push'" "forgejo.ref == 'refs/heads/main'" "needs.checks.outputs.release == 'true'"] {
    check ($release.if | str contains $condition) 'release must require a checked main version bump'
  }
  let candidate = $checks.steps | where {|step| ($step.id? | default '') == version } | first
  let publisher = $release.steps | last
  for step in [$candidate $publisher] {
    check ($step.env.BEFORE_SHA == '${{ forgejo.event.before }}' and $step.env.CHECKOUT_SHA == '${{ forgejo.sha }}') 'candidate and publisher must receive the same exact push range'
    check ($step.env.FORGEJO_EVENT_NAME == '${{ forgejo.event_name }}' and $step.env.FORGEJO_REF == '${{ forgejo.ref }}') 'publication context guards must receive the actual workflow event'
  }
  check ($workflow.concurrency.cancel-in-progress == "${{ forgejo.ref != 'refs/heads/main' }}") 'main publication must not be canceled by later pushes'
  let text = open --raw .forgejo/workflows/ci.yaml
  check (not ($text | str contains 'secrets.')) 'Forgejo CI must not require repository PAT secrets'
  check (not ($text | str contains 'apt-get')) 'tool installation must stay outside PR jobs'
}

def main [] {
  cd $project_dir
  test-package-contract
  test-ci-contract
  test-forgejo-ci-contract
  test-documentation-links
  print 'repository checks passed'
}
