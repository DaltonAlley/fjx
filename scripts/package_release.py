#!/usr/bin/env python3
"""Builds, packages and verifies the fjx release archives for every target."""

from __future__ import annotations

import json
import os
import re
import shutil
import stat
import sys
import tempfile

from release import (TOML_ERRORS, ArgumentParser, Failure, checked, child_env, cli_main, complete, decimal_int,
                     decode, is_int, lines, read_toml, release_version, remove, validate_versions)
from release_smoke import run_arm64_smoke_container

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
MANIFEST_PATH = os.path.join(os.path.dirname(SCRIPT_DIR), "release-targets.toml")
LINUX_AMD64_SMOKE_IMAGE = "docker.io/library/debian:bookworm-slim@sha256:88200866dfff7ea7f5cbcb6ec7c8a701889efe6fe859fe64d6990e4b07ea4171"
LINUX_ARM64_SMOKE_IMAGE = "docker.io/library/debian:bookworm-slim@sha256:6bd27d44e6c32a66bbd72d7cb2b76a8ae3497ec2e5274a81abd1b37f6013fa1f"
ARM64_QEMU_IMAGE = "docker.io/tonistiigi/binfmt:qemu-v10.2.3@sha256:400a4873b838d1b89194d982c45e5fb3cda4593fbfd7e08a02e76b03b21166f0"
ARM64_QEMU_VERSION_PATTERN = r"^qemu-aarch64 version 10\.2\.3(?: |$)"
ARM64_SMOKE_TIMEOUT = 30.0
ARM64_SMOKE_POLL_INTERVAL = 0.25
TARGET_FIELDS = ("triple", "os", "archive", "executable", "file_pattern", "runtime")


def fail(message: str):
    raise Failure(f"fjx package: {message}")


def release_targets() -> list[dict]:
    """Reads and checks release-targets.toml next to the scripts directory."""
    try:
        manifest = read_toml(MANIFEST_PATH)
    except TOML_ERRORS:
        fail("invalid target manifest")
    schema = manifest.get("schema")
    if not is_int(schema) or schema != 1:
        fail(f"unsupported target manifest schema {schema}")
    targets = manifest.get("targets")
    if not isinstance(targets, list) or len(targets) != 6:
        fail("target manifest must contain exactly six targets")
    for target in targets:
        if not isinstance(target, dict) or not all(isinstance(target.get(key), str) for key in TARGET_FIELDS):
            fail("target manifest entries must name " + ", ".join(TARGET_FIELDS))
        try:
            re.compile(target["file_pattern"])
        except re.error:
            fail(f"invalid file pattern for {target['triple']}")
    triples = [target["triple"] for target in targets]
    if len(set(triples)) != len(triples):
        fail("target manifest contains a duplicate triple")
    if "x86_64-apple-darwin" in triples:
        fail("x86_64 macOS must not be released")
    for target in targets:
        if target["archive"] not in ("tar.gz", "zip"):
            fail(f"invalid archive format for {target['triple']}")
        if target["runtime"] not in ("linux/amd64", "linux/arm64", "wine", "none"):
            fail(f"invalid runtime policy for {target['triple']}")
        if target["os"] == "macos" and target["runtime"] != "none":
            fail("macOS targets cannot claim runtime validation on Linux")
        if target["os"] == "windows" and (target["archive"] != "zip" or target["executable"] != "fjx.exe"):
            fail("Windows must ship fjx.exe in a zip")
        if target["os"] != "windows" and (target["archive"] != "tar.gz" or target["executable"] != "fjx"):
            fail(f"Unix target {target['triple']} must ship fjx in a tar.gz")
    return targets


def selected_targets(target: str) -> list[dict]:
    targets = release_targets()
    if target == "all":
        return targets
    selected = [entry for entry in targets if entry["triple"] == target]
    if not selected:
        fail(f"unsupported target {target}; use one from release-targets.toml")
    return selected


def archive_name(version: str, target: dict) -> str:
    return f"fjx-{version}-{target['triple']}.{target['archive']}"


def create_new(path: str, data: bytes) -> None:
    # Like Nushell's `save`: never overwrite an existing asset.
    try:
        with open(path, "xb") as file:
            file.write(data)
    except FileExistsError:
        fail(f"destination file already exists: {path}")


def package_seeded(source_bin: str, output_dir: str, version: str, target: dict, stage_root: str,
                   epoch: int) -> None:
    if not os.path.exists(source_bin):
        fail(f"missing executable {source_bin}")
    stage = os.path.join(stage_root, target["triple"])
    os.makedirs(stage, exist_ok=True)
    staged_bin = os.path.join(stage, target["executable"])
    shutil.copyfile(source_bin, staged_bin)
    os.chmod(staged_bin, 0o755)
    os.utime(staged_bin, (epoch, epoch))
    archive = archive_name(version, target)
    archive_path = os.path.join(output_dir, archive)
    if target["archive"] == "tar.gz":
        tarball = checked(["tar", "--directory", stage, "--owner=0", "--group=0", "--numeric-owner",
                           "--mtime", f"@{epoch}", "--sort=name", "-cf", "-", target["executable"]], capture=True)
        create_new(archive_path, checked(["gzip", "-n"], input=tarball, capture=True))
    else:
        checked(["zip", "-X", "-q", archive_path, target["executable"]], cwd=stage, env=child_env({"TZ": "UTC"}))
    checksum = checked(["sha256sum", archive], cwd=output_dir, capture=True)
    create_new(os.path.join(output_dir, f"{archive}.sha256"), checksum)


def extract_one(archive_path: str, target: dict, stage: str) -> None:
    name = os.path.basename(archive_path)
    if target["archive"] == "tar.gz":
        if lines(decode(checked(["tar", "-tzf", archive_path], capture=True))) != [target["executable"]]:
            fail(f"unexpected archive contents for {name}")
        checked(["tar", "-xzf", archive_path, "-C", stage])
    else:
        if lines(decode(checked(["unzip", "-Z1", archive_path], capture=True))) != [target["executable"]]:
            fail(f"unexpected archive contents for {name}")
        checked(["unzip", "-q", archive_path, "-d", stage])


def remove_container(runtime: str, container: str) -> None:
    if container:
        complete([runtime, "rm", "--force", container])


def file_description(path: str) -> str:
    return decode(checked(["file", "--brief", path], capture=True)).strip()


def smoke_linux_arm64(binary: str, platform: str, runtime: str) -> None:
    qemu_root = tempfile.mkdtemp()
    qemu = os.path.join(qemu_root, "qemu-aarch64")
    qemu_create = complete([runtime, "create", "--platform", "linux/amd64", ARM64_QEMU_IMAGE])
    if qemu_create.exit_code != 0:
        remove(qemu_root)
        fail(f"could not create pinned ARM64 QEMU source container: {qemu_create.stderr.strip()}")
    qemu_container = qemu_create.stdout.strip()
    if not qemu_container:
        remove(qemu_root)
        fail("container runtime returned no ARM64 QEMU source container ID")
    try:
        checked([runtime, "cp", f"{qemu_container}:/usr/bin/qemu-aarch64", qemu])
        os.chmod(qemu, 0o755)

        qemu_description = file_description(qemu)
        if not re.search(r"ELF 64-bit LSB.*x86-64.*(statically|static-pie) linked", qemu_description):
            fail(f"pinned ARM64 QEMU interpreter is not a static x86-64 executable: {qemu_description}")
        qemu_probe = complete([qemu, "--version"])
        qemu_version = (lines(qemu_probe.stdout) or [""])[0]
        if qemu_probe.exit_code != 0 or not re.search(ARM64_QEMU_VERSION_PATTERN, qemu_version):
            fail(f"unexpected pinned ARM64 QEMU interpreter version: {qemu_version}")

        # The pinned binfmt QEMU preserves argv[0], so direct invocation must
        # provide the guest path once as the executable and once as argv[0].
        smoke_create = complete([runtime, "create", "--platform", platform, "--entrypoint",
                                 "/usr/local/bin/qemu-aarch64", LINUX_ARM64_SMOKE_IMAGE, "-L", "/",
                                 "/usr/local/bin/fjx", "/usr/local/bin/fjx", "--help"])
        if smoke_create.exit_code != 0:
            fail(f"could not create ARM64 Linux smoke container for {platform}: {smoke_create.stderr.strip()}")
        smoke_container = smoke_create.stdout.strip()
        if not smoke_container:
            fail("container runtime returned no ARM64 Linux smoke container ID")
        run_arm64_smoke_container(runtime, smoke_container, platform, qemu, binary,
                                  ARM64_SMOKE_TIMEOUT, ARM64_SMOKE_POLL_INTERVAL)
    finally:
        remove_container(runtime, qemu_container)
        remove(qemu_root)


def pe_timestamp(binary: str) -> int:
    """Reads the COFF TimeDateStamp: 4 bytes at e_lfanew + 8, little-endian."""
    with open(binary, "rb") as file:
        data = file.read()
    name = os.path.basename(binary)
    if len(data) < 64 or data[0:2] != b"MZ":
        fail(f"{name} has no MZ header")
    offset = int.from_bytes(data[60:64], "little")
    if len(data) < offset + 12 or data[offset:offset + 4] != b"PE\0\0":
        fail(f"{name} has no PE header")
    return int.from_bytes(data[offset + 8:offset + 12], "little")


def verify_one(asset_dir: str, version: str, target: dict, runtime: str, wine: str, smoke: bool,
               epoch: int) -> None:
    archive = archive_name(version, target)
    archive_path = os.path.join(asset_dir, archive)
    checksum_path = os.path.join(asset_dir, f"{archive}.sha256")
    if not os.path.exists(archive_path):
        fail(f"missing {archive}")
    if not os.path.exists(checksum_path):
        fail(f"missing {archive}.sha256")
    with open(checksum_path, "rb") as file:
        checksum_line = decode(file.read()).strip()
    if not checksum_line.endswith(f"  {archive}"):
        fail(f"checksum names the wrong asset for {archive}")
    if complete(["sha256sum", "--check", "--strict", f"{archive}.sha256"], cwd=asset_dir).exit_code != 0:
        fail(f"checksum failed for {archive}")
    stage = tempfile.mkdtemp()
    try:
        extract_one(archive_path, target, stage)
        binary = os.path.join(stage, target["executable"])
        description = file_description(binary)
        if not re.search(target["file_pattern"], description):
            fail(f"wrong executable format for {target['triple']}: {description}")
        if target["os"] == "windows":
            # A build without SOURCE_DATE_EPOCH holds the link time instead.
            stamp = pe_timestamp(binary)
            if stamp != epoch:
                fail(f"PE timestamp {stamp} of {target['triple']} does not match the source date epoch {epoch}")
        elif "x" not in stat.filemode(os.lstat(binary).st_mode):
            fail(f"archive binary is not executable for {target['triple']}")
        if smoke and target["runtime"] == "linux/arm64":
            smoke_linux_arm64(binary, target["runtime"], runtime)
        elif smoke and target["runtime"].startswith("linux/"):
            with open(archive_path, "rb") as archive_file:
                checked([runtime, "run", "--rm", "--interactive", "--platform", target["runtime"],
                         LINUX_AMD64_SMOKE_IMAGE, "sh", "-ceu",
                         "mkdir /tmp/fjx-smoke; tar -xzf - -C /tmp/fjx-smoke; exec /tmp/fjx-smoke/fjx --help"],
                        stdin=archive_file)
        elif smoke and target["runtime"] == "wine" and wine:
            checked([wine, binary, "--help"])
    finally:
        remove(stage)


def listed_files(directory: str) -> list[str]:
    """Lists regular files like Nushell's `ls DIR | where type == file`: no hidden files or links."""
    try:
        with os.scandir(directory) as entries:
            return sorted(entry.name for entry in entries
                          if not entry.name.startswith(".") and entry.is_file(follow_symlinks=False))
    except (FileNotFoundError, NotADirectoryError):
        fail(f"asset directory does not exist: {directory}")


def verify_assets(asset_dir: str, version: str, source_date_epoch: int | None = None, runtime: str = "docker",
                  wine: str = "", smoke: bool = False) -> None:
    """Checks inventory, checksums, archive entries, formats, modes and the PE timestamp."""
    targets = release_targets()
    if smoke and not wine and any(target["runtime"] == "wine" for target in targets):
        fail("--wine is required for smoke verification because the release matrix contains a Wine target")
    if source_date_epoch is None and any(target["os"] == "windows" for target in targets):
        fail("--source-date-epoch is required to check the Windows PE timestamp")
    expected = sorted(name for target in targets
                      for name in (archive_name(version, target), f"{archive_name(version, target)}.sha256"))
    actual = listed_files(asset_dir)
    if actual != expected:
        fail(f"release asset inventory mismatch; expected {', '.join(expected)}; found {', '.join(actual)}")
    for target in targets:
        verify_one(asset_dir, version, target, runtime, wine, smoke,
                   0 if source_date_epoch is None else source_date_epoch)


def flatten_assets(platform_dir: str, asset_dir: str) -> None:
    if os.path.exists(asset_dir):
        fail(f"asset directory already exists: {asset_dir}")
    os.makedirs(asset_dir)
    for directory, subdirectories, files in os.walk(platform_dir):
        subdirectories.sort()
        for name in sorted(files):
            path = os.path.join(directory, name)
            if os.path.islink(path) or not os.path.isfile(path):
                continue
            destination = os.path.join(asset_dir, name)
            if os.path.exists(destination):
                fail(f"duplicate flattened asset {name}")
            shutil.copy(path, destination)


def build_target(runtime: str, project_dir: str, raw_root: str, target: dict, epoch: int) -> None:
    destination = os.path.join(raw_root, target["triple"])
    os.makedirs(destination, exist_ok=True)
    arguments = ["--file", os.path.join(project_dir, "packaging", "release.Containerfile"),
                 "--build-arg", f"TARGET={target['triple']}", "--build-arg", f"EXECUTABLE={target['executable']}",
                 "--build-arg", f"SOURCE_DATE_EPOCH={epoch}", "--output", f"type=local,dest={destination}",
                 project_dir]
    for attempt in (1, 2):
        if os.path.basename(runtime) == "podman":
            result = complete([runtime, "build", *arguments])
        else:
            result = complete([runtime, "buildx", "build", *arguments])
        sys.stdout.write(result.stdout)
        sys.stdout.flush()
        sys.stderr.write(result.stderr)
        if result.exit_code == 0:
            return
        if attempt == 2:
            fail(f"cross build failed twice for {target['triple']}")
        print(f"cross build failed for {target['triple']}; retrying once", file=sys.stderr, flush=True)


def package(output_dir: str, target_name: str, epoch: int, build_runtime: str = "") -> None:
    """Packages one target or `all` into OUTPUT_DIR, from FJX_PACKAGE_BIN_ROOT or a container build."""
    if os.path.realpath(output_dir) == "/":
        fail("unsafe output directory")
    os.makedirs(output_dir, exist_ok=True)
    output_dir = os.path.realpath(output_dir)
    version = release_version()
    validate_versions()
    targets = selected_targets(target_name)
    project_dir = os.path.dirname(SCRIPT_DIR)
    stage_root = tempfile.mkdtemp()
    raw_root = tempfile.mkdtemp()
    try:
        for target in targets:
            source_root = os.environ.get("FJX_PACKAGE_BIN_ROOT", "")
            if not source_root:
                if not build_runtime:
                    fail("a build runtime is required when FJX_PACKAGE_BIN_ROOT is unset")
                build_target(build_runtime, project_dir, raw_root, target, epoch)
                source_root = raw_root
            package_seeded(os.path.join(source_root, target["triple"], target["executable"]), output_dir,
                           version, target, stage_root, epoch)
    finally:
        remove(stage_root)
        remove(raw_root)


def absolute(path: str) -> str:
    return os.path.abspath(os.path.expanduser(path))


def run(argv: list[str]) -> None:
    command = argv[0] if argv else ""
    if command == "build":
        parser = ArgumentParser(prog="package_release.py build", description="Build and package all targets")
        parser.add_argument("output_dir", type=absolute)
        parser.add_argument("--runtime", default="docker")
        parser.add_argument("--source-date-epoch", type=decimal_int, required=True)
        arguments = parser.parse_args(argv[1:])
        package(arguments.output_dir, "all", arguments.source_date_epoch, arguments.runtime)
    elif command == "flatten":
        parser = ArgumentParser(prog="package_release.py flatten", description="Flatten per-platform assets")
        parser.add_argument("platform_dir", type=absolute)
        parser.add_argument("asset_dir", type=absolute)
        arguments = parser.parse_args(argv[1:])
        flatten_assets(arguments.platform_dir, arguments.asset_dir)
    elif command == "verify":
        parser = ArgumentParser(prog="package_release.py verify", description="Verify release archives")
        parser.add_argument("asset_dir", type=absolute)
        parser.add_argument("version")
        parser.add_argument("--source-date-epoch", type=decimal_int)
        parser.add_argument("--runtime", default="docker")
        parser.add_argument("--wine", default="")
        parser.add_argument("--smoke", action="store_true")
        arguments = parser.parse_args(argv[1:])
        verify_assets(arguments.asset_dir, arguments.version, arguments.source_date_epoch, arguments.runtime,
                      arguments.wine, arguments.smoke)
    elif command == "matrix":
        ArgumentParser(prog="package_release.py matrix").parse_args(argv[1:])
        print(json.dumps(release_targets(), indent=2, ensure_ascii=False))
    else:
        parser = ArgumentParser(prog="package_release.py",
                                description="Package release archives: OUTPUT_DIR [TARGET|all]; "
                                            "subcommands: build, flatten, verify, matrix")
        parser.add_argument("output_dir", type=absolute)
        parser.add_argument("target", nargs="?", default="all")
        parser.add_argument("--source-date-epoch", type=decimal_int, default=0)
        arguments = parser.parse_args(argv)
        if os.environ.get("FJX_PACKAGE_BIN_ROOT", ""):
            build_runtime = ""
        else:
            build_runtime = os.environ.get("FJX_CONTAINER_RUNTIME", "podman")
        package(arguments.output_dir, arguments.target, arguments.source_date_epoch, build_runtime)


if __name__ == "__main__":
    sys.exit(cli_main(run))
