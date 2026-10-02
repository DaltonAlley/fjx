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

def test-issue-forms [] {
  for file in [bug_report.yml feature_request.yml] {
    let form = open ('.github/ISSUE_TEMPLATE' | path join $file)
    check (($form.name | is-not-empty) and ($form.description | is-not-empty)) $"($file) needs a name and description"
    let fields = $form.body | where type != markdown
    let ids = $fields | get id
    check (($ids | uniq | length) == ($ids | length)) $"($file) has duplicate field IDs"
    check ($fields | any {|field| $field.validations.required? | default false }) $"($file) must request required reproduction or use-case information"
    for field in $fields {
      check ($field.attributes.label | is-not-empty) $"($file) has an unlabeled input"
    }
  }
  let config = open .github/ISSUE_TEMPLATE/config.yml
  check $config.blank_issues_enabled 'keep a general help and private-channel request route open'
  check ($config.contact_links | any {|link| $link.url | str ends-with '/SECURITY.md' }) 'route security reports to the security policy'
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

def main [] {
  cd $project_dir
  test-package-contract
  test-ci-contract
  test-issue-forms
  test-documentation-links
  print 'repository checks passed'
}
