#!/usr/bin/env nu

use release.nu [release-version validate-versions]
use release-smoke.nu [run-arm64-smoke-container]

const script_dir = path self .
const manifest_path = path self ../release-targets.toml
const linux_amd64_smoke_image = 'docker.io/library/debian:bookworm-slim@sha256:88200866dfff7ea7f5cbcb6ec7c8a701889efe6fe859fe64d6990e4b07ea4171'
const linux_arm64_smoke_image = 'docker.io/library/debian:bookworm-slim@sha256:6bd27d44e6c32a66bbd72d7cb2b76a8ae3497ec2e5274a81abd1b37f6013fa1f'
const arm64_qemu_image = 'docker.io/tonistiigi/binfmt:qemu-v10.2.3@sha256:400a4873b838d1b89194d982c45e5fb3cda4593fbfd7e08a02e76b03b21166f0'
const arm64_qemu_version_pattern = '^qemu-aarch64 version 10\.2\.3(?: |$)'
const arm64_smoke_timeout = 30sec
const arm64_smoke_poll_interval = 250ms

def fail [message: string] { error make {msg: $"fjx package: ($message)"} }

export def release-targets [] {
  let manifest = (open --raw $manifest_path | from toml)
  if $manifest.schema != 1 { fail $"unsupported target manifest schema ($manifest.schema)" }
  let targets = $manifest.targets
  if ($targets | length) != 6 { fail "target manifest must contain exactly six targets" }
  let triples = ($targets | get triple)
  if (($triples | uniq | length) != ($triples | length)) { fail "target manifest contains a duplicate triple" }
  if 'x86_64-apple-darwin' in $triples { fail "x86_64 macOS must not be released" }
  for target in $targets {
    if $target.archive not-in [tar.gz zip] { fail $"invalid archive format for ($target.triple)" }
    if $target.runtime not-in [linux/amd64 linux/arm64 wine none] { fail $"invalid runtime policy for ($target.triple)" }
    if $target.os == macos and $target.runtime != none { fail "macOS targets cannot claim runtime validation on Linux" }
    if $target.os == windows and ($target.archive != zip or $target.executable != fjx.exe) { fail "Windows must ship fjx.exe in a zip" }
    if $target.os != windows and ($target.archive != tar.gz or $target.executable != fjx) { fail $"Unix target ($target.triple) must ship fjx in a tar.gz" }
  }
  $targets
}

def selected-targets [target: string] {
  let targets = (release-targets)
  if $target == all { return $targets }
  let selected = ($targets | where triple == $target)
  if ($selected | is-empty) { fail $"unsupported target ($target); use one from release-targets.toml" }
  $selected
}

def archive-name [version: string, target: record] {
  $"fjx-($version)-($target.triple).($target.archive)"
}

def package-seeded [source_bin: path, output_dir: path, version: string, target: record, stage_root: path, epoch: int] {
  if not ($source_bin | path exists) { fail $"missing executable ($source_bin)" }
  let stage = ($stage_root | path join $target.triple)
  mkdir $stage
  let staged_bin = ($stage | path join $target.executable)
  cp $source_bin $staged_bin
  chmod 0755 $staged_bin
  ^touch --date $"@($epoch)" $staged_bin
  let archive = (archive-name $version $target)
  let archive_path = ($output_dir | path join $archive)
  if $target.archive == tar.gz {
    do { ^tar --directory $stage --owner=0 --group=0 --numeric-owner --mtime $"@($epoch)" --sort=name -cf - $target.executable } | ^gzip -n | save --raw $archive_path
  } else {
    with-env {TZ: UTC} { do { cd $stage; ^zip -X -q $archive_path $target.executable } }
  }
  do { cd $output_dir; ^sha256sum $archive } | save --raw ($output_dir | path join $"($archive).sha256")
}

def extract-one [archive_path: path, target: record, stage: path] {
  if $target.archive == tar.gz {
    let contents = (^tar -tzf $archive_path | lines)
    if $contents != [$target.executable] { fail $"unexpected archive contents for ($archive_path | path basename)" }
    ^tar -xzf $archive_path -C $stage
  } else {
    let contents = (^unzip -Z1 $archive_path | lines)
    if $contents != [$target.executable] { fail $"unexpected archive contents for ($archive_path | path basename)" }
    ^unzip -q $archive_path -d $stage
  }
}

def remove-container [container_runtime: string, container: string] {
  if ($container | is-not-empty) {
    do { ^$container_runtime rm --force $container } | complete | ignore
  }
}

def smoke-linux-arm64 [binary: path, platform: string, container_runtime: string] {
  let qemu_root = (^mktemp -d | str trim)
  let qemu = ($qemu_root | path join qemu-aarch64)
  let qemu_create = (do {
    ^$container_runtime create --platform linux/amd64 $arm64_qemu_image
  } | complete)
  if $qemu_create.exit_code != 0 {
    rm -rf $qemu_root
    fail $"could not create pinned ARM64 QEMU source container: ($qemu_create.stderr | str trim)"
  }
  let qemu_container = ($qemu_create.stdout | str trim)
  if ($qemu_container | is-empty) {
    rm -rf $qemu_root
    fail "container runtime returned no ARM64 QEMU source container ID"
  }
  try {
    ^$container_runtime cp $"($qemu_container):/usr/bin/qemu-aarch64" $qemu
    chmod 0755 $qemu

    let qemu_description = (^file --brief $qemu | str trim)
    if not ($qemu_description =~ 'ELF 64-bit LSB.*x86-64.*(statically|static-pie) linked') {
      fail $"pinned ARM64 QEMU interpreter is not a static x86-64 executable: ($qemu_description)"
    }
    let qemu_probe = (do { ^$qemu --version } | complete)
    let qemu_version = ($qemu_probe.stdout | lines | first | default '')
    if $qemu_probe.exit_code != 0 or not ($qemu_version =~ $arm64_qemu_version_pattern) {
      fail $"unexpected pinned ARM64 QEMU interpreter version: ($qemu_version)"
    }

    let smoke_create = (do {
      # The pinned binfmt QEMU preserves argv[0], so direct invocation must
      # provide the guest path once as the executable and once as argv[0].
      ^$container_runtime create --platform $platform --entrypoint /usr/local/bin/qemu-aarch64 $linux_arm64_smoke_image -L / /usr/local/bin/fjx /usr/local/bin/fjx --help
    } | complete)
    if $smoke_create.exit_code != 0 {
      fail $"could not create ARM64 Linux smoke container for ($platform): ($smoke_create.stderr | str trim)"
    }
    let smoke_container = ($smoke_create.stdout | str trim)
    if ($smoke_container | is-empty) { fail "container runtime returned no ARM64 Linux smoke container ID" }
    run-arm64-smoke-container $container_runtime $smoke_container $platform $qemu $binary $arm64_smoke_timeout $arm64_smoke_poll_interval
  } catch {|error|
    remove-container $container_runtime $qemu_container
    rm -rf $qemu_root
    error make $error.raw
  }
  remove-container $container_runtime $qemu_container
  rm -rf $qemu_root
}

def verify-one [asset_dir: path, version: string, target: record, container_runtime: string, wine: string, smoke: bool] {
  let archive = (archive-name $version $target)
  let archive_path = ($asset_dir | path join $archive)
  let checksum_path = ($asset_dir | path join $"($archive).sha256")
  if not ($archive_path | path exists) { fail $"missing ($archive)" }
  if not ($checksum_path | path exists) { fail $"missing ($archive).sha256" }
  let checksum_line = (open --raw $checksum_path | str trim)
  if not ($checksum_line | str ends-with $"  ($archive)") { fail $"checksum names the wrong asset for ($archive)" }
  let checksum = (do { cd $asset_dir; ^sha256sum --check --strict $"($archive).sha256" } | complete)
  if $checksum.exit_code != 0 { fail $"checksum failed for ($archive)" }
  let stage = (^mktemp -d | str trim)
  try {
    extract-one $archive_path $target $stage
    let binary = ($stage | path join $target.executable)
    let description = (^file --brief $binary | str trim)
    if not ($description =~ $target.file_pattern) { fail $"wrong executable format for ($target.triple): ($description)" }
    if $target.os != windows {
      let mode = (^stat -c '%A' $binary | str trim)
      if not ($mode | str contains 'x') { fail $"archive binary is not executable for ($target.triple)" }
    }
    if $smoke and $target.runtime == 'linux/arm64' {
      smoke-linux-arm64 $binary $target.runtime $container_runtime
    } else if $smoke and ($target.runtime | str starts-with 'linux/') {
      open --raw $archive_path | ^$container_runtime run --rm --interactive --platform $target.runtime $linux_amd64_smoke_image sh -ceu 'mkdir /tmp/fjx-smoke; tar -xzf - -C /tmp/fjx-smoke; exec /tmp/fjx-smoke/fjx --help'
    } else if $smoke and $target.runtime == wine and ($wine | is-not-empty) {
      ^$wine $binary --help
    }
  } catch {|error|
    rm -rf $stage
    error make $error.raw
  }
  rm -rf $stage
}

export def verify-assets [asset_dir: path, version: string, --runtime: string = docker, --wine: string = '', --smoke] {
  let targets = (release-targets)
  if $smoke and ($wine | is-empty) and ($targets | any {|target| $target.runtime == wine }) {
    fail "--wine is required for smoke verification because the release matrix contains a Wine target"
  }
  let expected = ($targets | each {|target| let archive = (archive-name $version $target); [$archive $"($archive).sha256"] } | flatten | sort)
  let actual = (ls $asset_dir | where type == file | get name | each {|file| $file | path basename } | sort)
  if $actual != $expected { fail $"release asset inventory mismatch; expected ($expected | str join ', '); found ($actual | str join ', ')" }
  for target in $targets { verify-one $asset_dir $version $target $runtime $wine $smoke }
}

export def flatten-assets [platform_dir: path, asset_dir: path] {
  if ($asset_dir | path exists) { fail $"asset directory already exists: ($asset_dir)" }
  mkdir $asset_dir
  let files = (glob ($platform_dir | path join '**' '*') | where {|path| ($path | path type) == file })
  for file in $files {
    let destination = ($asset_dir | path join ($file | path basename))
    if ($destination | path exists) { fail $"duplicate flattened asset ($destination | path basename)" }
    cp $file $destination
  }
}

def build-target [runtime: string, project_dir: path, raw_root: path, target: record] {
  let destination = ($raw_root | path join $target.triple)
  mkdir $destination
  let arguments = [--file ($project_dir | path join packaging release.Containerfile) --build-arg $"TARGET=($target.triple)" --build-arg $"EXECUTABLE=($target.executable)" --output $"type=local,dest=($destination)" $project_dir]
  for attempt in 1..2 {
    let result = if ($runtime | path basename) == podman {
      do { ^$runtime build ...$arguments } | complete
    } else {
      do { ^$runtime buildx build ...$arguments } | complete
    }
    print --no-newline $result.stdout
    print --stderr --no-newline $result.stderr
    if $result.exit_code == 0 { return }
    if $attempt == 2 {
      fail $"cross build failed twice for ($target.triple)"
    }
    print --stderr $"cross build failed for ($target.triple); retrying once"
  }
}

def package [output_dir: path, target_name: string, epoch: int, build_runtime: string = ''] {
  if ($output_dir | path expand) == '/' { fail "unsafe output directory" }
  mkdir $output_dir
  let output_dir = ($output_dir | path expand)
  let version = (release-version)
  validate-versions | ignore
  let targets = (selected-targets $target_name)
  let project_dir = ($script_dir | path dirname)
  let stage_root = (^mktemp -d | str trim)
  let raw_root = (^mktemp -d | str trim)
  try {
    for target in $targets {
      let source_root = if ($env.FJX_PACKAGE_BIN_ROOT? | is-not-empty) {
        $env.FJX_PACKAGE_BIN_ROOT
      } else {
        if ($build_runtime | is-empty) { fail "a build runtime is required when FJX_PACKAGE_BIN_ROOT is unset" }
        build-target $build_runtime $project_dir $raw_root $target
        $raw_root
      }
      package-seeded ($source_root | path join $target.triple $target.executable) $output_dir $version $target $stage_root $epoch
    }
  } catch {|error|
    rm -rf $stage_root $raw_root
    error make $error.raw
  }
  rm -rf $stage_root $raw_root
}

def main [output_dir: path, target: string = all, --source-date-epoch: int = 0] {
  let build_runtime = if ($env.FJX_PACKAGE_BIN_ROOT? | is-not-empty) { '' } else { $env.FJX_CONTAINER_RUNTIME? | default podman }
  package $output_dir $target $source_date_epoch $build_runtime
}

def "main build" [output_dir: path, --runtime: string = docker, --source-date-epoch: int] {
  package $output_dir all $source_date_epoch $runtime
}

def "main flatten" [platform_dir: path, asset_dir: path] { flatten-assets $platform_dir $asset_dir }

def "main verify" [asset_dir: path, version: string, --runtime: string = docker, --wine: string = '', --smoke] {
  verify-assets $asset_dir $version --runtime=$runtime --wine=$wine --smoke=$smoke
}

def "main matrix" [] { release-targets | to json }
