#!/usr/bin/env nu

def fail [message: string] { error make {msg: $"fjx package: ($message)"} }

def remove-container [container_runtime: string, container: string] {
  if ($container | is-not-empty) {
    do { ^$container_runtime rm --force $container } | complete | ignore
  }
}

def output [result: record] {
  [$result.stdout $result.stderr] | each { str trim } | where { is-not-empty } | str join (char nl)
}

export def run-arm64-smoke-container [
  container_runtime: string
  smoke_container: string
  platform: string
  qemu: path
  binary: path
  timeout: duration
  poll_interval: duration
] {
  if $timeout <= 0sec { fail "ARM64 Linux smoke timeout must be positive" }
  if $poll_interval <= 0sec { fail "ARM64 Linux smoke poll interval must be positive" }
  try {
    for copy in [
      {source: $qemu, destination: $"($smoke_container):/usr/local/bin/qemu-aarch64"}
      {source: $binary, destination: $"($smoke_container):/usr/local/bin/fjx"}
    ] {
      let copied = (do { ^$container_runtime cp $copy.source $copy.destination } | complete)
      if $copied.exit_code != 0 {
        fail $"could not prepare explicit-QEMU ARM64 Linux smoke for ($platform): ($copied.stderr | str trim)"
      }
    }
    let started = (do { ^$container_runtime start $smoke_container } | complete)
    if $started.exit_code != 0 {
      fail $"could not start explicit-QEMU ARM64 Linux smoke for ($platform): ($started.stderr | str trim)"
    }
    let deadline = ((date now) + $timeout)
    loop {
      let inspected = (do { ^$container_runtime inspect $smoke_container } | complete)
      if $inspected.exit_code != 0 {
        fail $"could not inspect explicit-QEMU ARM64 Linux smoke for ($platform): ($inspected.stderr | str trim)"
      }
      let inspections = (try { $inspected.stdout | from json } catch {
        fail $"container runtime returned invalid ARM64 Linux smoke state for ($platform)"
      })
      let inspection_type = ($inspections | describe)
      if not ($inspection_type | str starts-with 'list') and not ($inspection_type | str starts-with 'table') {
        fail $"container runtime returned invalid ARM64 Linux smoke state for ($platform)"
      }
      if ($inspections | length) != 1 {
        fail $"container runtime returned an unexpected ARM64 Linux smoke state count for ($platform)"
      }
      let state = ($inspections | first | get State)
      let status = ($state.Status | into string)
      if $status in [exited dead] {
        let logs = (do { ^$container_runtime logs $smoke_container } | complete)
        if $logs.exit_code != 0 {
          fail $"could not read explicit-QEMU ARM64 Linux smoke logs for ($platform): ($logs.stderr | str trim)"
        }
        let exit_code = (try { $state.ExitCode | into int } catch {
          fail $"container runtime returned a nonnumeric ARM64 Linux smoke exit code for ($platform)"
        })
        if $status != exited or $exit_code != 0 {
          fail $"explicit-QEMU ARM64 Linux runtime smoke failed for ($platform) with status ($status) and exit code ($exit_code); logs: (output $logs)"
        }
        break
      }
      if $status not-in [created running] {
        fail $"explicit-QEMU ARM64 Linux runtime smoke entered unexpected status ($status) for ($platform)"
      }
      if (date now) >= $deadline {
        let timed_out_logs = (do { ^$container_runtime logs $smoke_container } | complete)
        fail $"explicit-QEMU ARM64 Linux runtime smoke timed out after ($timeout) for ($platform) in status ($status); logs: (output $timed_out_logs)"
      }
      sleep $poll_interval
    }
  } catch {|error|
    remove-container $container_runtime $smoke_container
    error make {msg: $error.msg}
  }
  remove-container $container_runtime $smoke_container
}
