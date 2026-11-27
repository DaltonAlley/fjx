"""Tests for scripts/release.py, scripts/package_release.py and scripts/release_smoke.py.

Run: python3 -m unittest discover --start-directory tests --pattern 'test_*.py'
Needs git, tar, gzip, zip, unzip, file and GNU coreutils on PATH.
"""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import sys
import tomllib
import unittest
from unittest import mock

from script_support import (FIXTURES, ROOT, SCRIPTS, TempTestCase, base_env, install_tool, read, real_program,
                            run_script, with_path, write)

import package_release
import release
import release_smoke

sys.path.insert(0, FIXTURES)
from forgejo_release_tools import pe_header  # noqa: E402

RELEASE_TARGETS = os.path.join(ROOT, "release-targets.toml")
RELEASE_CONTAINERFILE = os.path.join(ROOT, "packaging", "release.Containerfile")
REVISION = "a" * 40
EPOCH = 1700000000
FORMATS = {
    "x86_64-unknown-linux-gnu": "ELF 64-bit LSB pie executable, x86-64, dynamically linked",
    "aarch64-unknown-linux-gnu": "ELF 64-bit LSB pie executable, ARM aarch64, dynamically linked",
    "x86_64-unknown-linux-musl": "ELF 64-bit LSB executable, x86-64, statically linked",
    "aarch64-unknown-linux-musl": "ELF 64-bit LSB executable, ARM aarch64, static-pie linked",
    "aarch64-apple-darwin": "Mach-O 64-bit arm64 executable",
    "x86_64-pc-windows-gnu": "PE32+ executable for MS Windows 6.00 (console), x86-64, 6 sections",
}


def repository_version() -> str:
    with open(os.path.join(ROOT, "release.toml"), "rb") as file:
        return tomllib.load(file)["version"]


def targets() -> list[dict]:
    return package_release.release_targets()


def expected_asset_names(version: str) -> list[str]:
    names = []
    for target in targets():
        archive = f"fjx-{version}-{target['triple']}.{target['archive']}"
        names += [archive, f"{archive}.sha256"]
    return names


def make_fixture(root: str, version: str = "1.2.3") -> str:
    project = os.path.join(root, "fjx")
    write(os.path.join(project, "release.toml"), f'version = "{version}"\n')
    write(os.path.join(project, "Cargo.toml"), f'[package]\nname = "fjx"\nversion = "{version}"\n')
    write(os.path.join(project, "Cargo.lock"), f'version = 4\n\n[[package]]\nname = "fjx"\nversion = "{version}"\n')
    shutil.copy(RELEASE_TARGETS, os.path.join(project, "release-targets.toml"))
    return project


def seed_binaries(root: str, formats: bool = False, epoch: int = EPOCH) -> str:
    """One executable per target. With formats, each embeds a `# format:` line for the fake `file`."""
    for target in targets():
        path = os.path.join(root, target["triple"], target["executable"])
        script = "#!/bin/sh\necho fjx\n"
        if formats:
            script = f"#!/bin/sh\n# format: {FORMATS[target['triple']]}\necho fjx\n"
        data = script.encode()
        if formats and target["os"] == "windows":
            data = pe_header(epoch) + b"\n" + data
        write(path, data, None if target["os"] == "windows" else 0o755)
    return root


def archive_entries(path: str) -> list[str]:
    program = ["tar", "-tzf"] if path.endswith(".tar.gz") else ["unzip", "-Z1"]
    return subprocess.run([*program, path], capture_output=True, text=True, check=True).stdout.splitlines()


def read_lines(path: str) -> list:
    with open(path, encoding="utf-8") as file:
        return [json.loads(line) for line in file if line.strip()]


def run_release(root: str, arguments: list[str], env: dict | None = None, cwd: str | None = None):
    return run_script("release.py", arguments, {"RELEASE_WORKSPACE_ROOT": root, **(env or {})}, cwd=cwd)


class VersionTests(TempTestCase):
    def assert_versions(self, root: str, version: str):
        validated = run_release(root, ["validate"])
        self.assertSucceeded(validated)
        self.assertEqual(validated.stdout.strip(), version)

    def test_standalone_root(self):
        make_fixture(self.tmp)
        standalone = self.path("renamed-checkout")
        os.rename(self.path("fjx"), standalone)
        scripts = os.path.join(standalone, "scripts")
        os.makedirs(scripts)
        shutil.copy(os.path.join(SCRIPTS, "release.py"), scripts)
        # Run outside the checkout and with no parent/fjx directory. An empty
        # override must behave like an unset override, not resolve relative to cwd.
        script = os.path.join(scripts, "release.py")
        validated = run_script(script, ["validate"], {"RELEASE_WORKSPACE_ROOT": ""}, cwd=self.tmp)
        self.assertSucceeded(validated)
        self.assertEqual(validated.stdout.strip(), "1.2.3")
        prepared = run_script(script, ["prepare", "patch"], {"RELEASE_WORKSPACE_ROOT": ""}, cwd=self.tmp)
        self.assertSucceeded(prepared)
        for name in ("release.toml", "Cargo.toml", "Cargo.lock"):
            self.assertIn(b'version = "1.2.4"', read(os.path.join(standalone, name)), name)

    def test_version_validation_and_tags(self):
        project = make_fixture(self.tmp)
        self.assert_versions(self.tmp, "1.2.3")
        self.assertEqual(run_release(self.tmp, ["version"]).stdout, "1.2.3\n")
        self.assertSucceeded(run_release(self.tmp, ["validate-tag", "fjx/v1.2.3"]))
        self.assertFailed(run_release(self.tmp, ["validate-tag", "fjx/v1.2.4"]), "tag must be fjx/v1.2.3")
        write(os.path.join(project, "release.toml"), 'version = "01.2.3"\n')
        self.assertFailed(run_release(self.tmp, ["validate"]), "invalid version in release.toml")

    def test_version_files_need_exact_types(self):
        cases = {
            "release.toml": ['version = 1\n', 'version = "1.2.3\\n"\n', 'version = ["1.2.3"]\n', 'other = 1\n',
                             'version = "1.2.3"\nversion = "1.2.3"\n'],
            "Cargo.toml": ['[package]\nname = "fjx"\nversion = 123\n', 'package = "1.2.3"\n',
                           '[[package]]\nversion = "1.2.3"\n'],
            "Cargo.lock": ['[package]\nname = "fjx"\nversion = "1.2.3"\n', '[[package]]\nname = "other"\n',
                           '[[package]]\nname = "fjx"\nversion = "1.2.3"\n[[package]]\nname = "fjx"\nversion = "1.2.3"\n',
                           '[[package]]\nname = "fjx"\nversion = 1.2\n'],
        }
        for name, texts in cases.items():
            for text in texts:
                with self.subTest(file=name, text=text):
                    make_fixture(self.tmp)
                    write(self.path("fjx", name), text)
                    result = run_release(self.tmp, ["validate"])
                    self.assertFailed(result, "invalid")
                    self.assertIn(name, result.stderr)
        make_fixture(self.tmp)
        write(self.path("fjx", "Cargo.toml"), '[package]\nname = "fjx"\nversion = "1.2.4"\n')
        self.assertFailed(run_release(self.tmp, ["validate"]),
                          "Cargo.toml version 1.2.4 does not match release.toml 1.2.3")
        os.remove(self.path("fjx", "Cargo.lock"))
        make_fixture(self.tmp)
        os.remove(self.path("fjx", "Cargo.lock"))
        self.assertFailed(run_release(self.tmp, ["validate"]), "missing Cargo.lock")

    def test_dry_run_and_success(self):
        project = make_fixture(self.tmp)
        before = read(os.path.join(project, "release.toml"))
        dry = run_release(self.tmp, ["prepare", "--dry-run", "minor"])
        self.assertSucceeded(dry)
        self.assertEqual(dry.stdout, "project=fjx\ncurrent_version=1.2.3\nproposed_version=1.3.0\n"
                                     "tag=fjx/v1.3.0\ndry_run=true\n")
        self.assertEqual(read(os.path.join(project, "release.toml")), before, "dry run changed a file")
        self.assertSucceeded(run_release(self.tmp, ["prepare", "major", "--dry-run"]))
        self.assertFailed(run_release(self.tmp, ["prepare", "huge"]), "bump must be patch, minor, or major")
        prepared = run_release(self.tmp, ["prepare", "patch"])
        self.assertSucceeded(prepared)
        self.assertIn("dry_run=false", prepared.stdout)
        self.assert_versions(self.tmp, "1.2.4")
        self.assertEqual([name for name in os.listdir(project) if name.startswith(".release-prepare")], [])

    def test_prepare_rewrites_only_the_fjx_entries(self):
        project = make_fixture(self.tmp)
        write(os.path.join(project, "Cargo.toml"),
              '[package]\nname = "fjx"\nversion = "1.2.3"\n\n[dependencies]\nserde = { version = "1.2.3" }\n')
        write(os.path.join(project, "Cargo.lock"),
              'version = 4\n\n[[package]]\nname = "aaa"\nversion = "1.2.3"\n\n'
              '[[package]]\nname = "fjx"\nversion = "1.2.3"\n\n[[package]]\nname = "zzz"\nversion = "1.2.3"\n')
        self.assertSucceeded(run_release(self.tmp, ["prepare", "patch"]))
        self.assertEqual(read(os.path.join(project, "Cargo.toml")),
                         b'[package]\nname = "fjx"\nversion = "1.2.4"\n\n[dependencies]\nserde = { version = "1.2.3" }\n')
        self.assertEqual(read(os.path.join(project, "Cargo.lock")),
                         b'version = 4\n\n[[package]]\nname = "aaa"\nversion = "1.2.3"\n\n'
                         b'[[package]]\nname = "fjx"\nversion = "1.2.4"\n\n[[package]]\nname = "zzz"\nversion = "1.2.3"\n')

    def test_prepare_rollbacks(self):
        files = ("release.toml", "Cargo.toml", "Cargo.lock")
        for environment in ({"RELEASE_PREPARE_FAIL_AFTER_MOVE": "1"}, {"RELEASE_PREPARE_FAIL_AFTER_MOVE": "2"},
                            {"RELEASE_PREPARE_FAIL_AFTER_MOVE": "3"}, {"RELEASE_PREPARE_FAIL_VALIDATION": "true"}):
            with self.subTest(environment=environment):
                project = make_fixture(self.tmp)
                originals = [read(os.path.join(project, name)) for name in files]
                failed = run_release(self.tmp, ["prepare", "patch"], environment)
                self.assertFailed(failed, "injected")
                for name, original in zip(files, originals):
                    self.assertEqual(read(os.path.join(project, name)), original, f"{name} was not rolled back")
                self.assertEqual([name for name in os.listdir(project) if name.startswith(".release-prepare")], [])


class PackageTests(TempTestCase):
    def package(self, arguments: list[str], env: dict | None = None):
        return run_script("package_release.py", arguments, env)

    def test_package_targets_and_seeded_assets(self):
        bins = seed_binaries(self.mkdir("bins"))
        output, duplicate = self.path("output"), self.path("duplicate")
        environment = {"FJX_PACKAGE_BIN_ROOT": bins}
        self.assertFailed(self.package([output, "bad-target"], environment), "unsupported target bad-target")
        self.assertSucceeded(self.package([output, "all", "--source-date-epoch", str(EPOCH)], environment))
        self.assertSucceeded(self.package([duplicate, "--source-date-epoch", str(EPOCH)], environment))
        version = repository_version()
        for target in targets():
            archive = f"fjx-{version}-{target['triple']}.{target['archive']}"
            self.assertEqual(archive_entries(os.path.join(output, archive)), [target["executable"]], archive)
            checked = subprocess.run(["sha256sum", "--check", f"{archive}.sha256"], cwd=output, capture_output=True)
            self.assertEqual(checked.returncode, 0, f"bad checksum for {archive}")
            self.assertEqual(read(os.path.join(output, archive)), read(os.path.join(duplicate, archive)),
                             f"non-deterministic archive {archive}")
        self.assertEqual(len(os.listdir(output)), 12, "six targets did not produce exactly twelve assets")
        # Like Nushell's `save`, packaging never overwrites an existing asset.
        self.assertFailed(self.package([output, "x86_64-unknown-linux-gnu"], environment), "already exists")
        self.assertFailed(self.package(["/", "all"], environment), "unsafe output directory")

    def test_single_target_and_missing_runtime(self):
        bins = seed_binaries(self.mkdir("bins"))
        output = self.path("single")
        self.assertSucceeded(self.package([output, "aarch64-apple-darwin"], {"FJX_PACKAGE_BIN_ROOT": bins}))
        self.assertEqual(sorted(os.listdir(output)), [f"fjx-{repository_version()}-aarch64-apple-darwin.tar.gz",
                                                      f"fjx-{repository_version()}-aarch64-apple-darwin.tar.gz.sha256"])
        self.assertFailed(self.package([self.path("none")], {"FJX_CONTAINER_RUNTIME": ""}),
                          "a build runtime is required when FJX_PACKAGE_BIN_ROOT is unset")
        os.remove(os.path.join(bins, "x86_64-unknown-linux-gnu", "fjx"))
        self.assertFailed(self.package([self.path("missing")], {"FJX_PACKAGE_BIN_ROOT": bins}),
                          "missing executable")

    def test_default_runtime_and_asset_checks(self):
        bins = seed_binaries(self.mkdir("bins"))
        assets = self.path("assets")
        self.assertSucceeded(self.package([assets, "all"], {"FJX_PACKAGE_BIN_ROOT": bins}))
        version = repository_version()
        self.assertFailed(self.package(["verify", assets, version]), "--source-date-epoch is required")
        epoch = ["--source-date-epoch", str(EPOCH)]
        self.assertFailed(self.package(["verify", assets, version, *epoch]), "wrong executable format")
        write(os.path.join(assets, "unexpected.txt"), "")
        extra = self.package(["verify", assets, version, *epoch])
        self.assertFailed(extra, "release asset inventory mismatch")
        self.assertIn("unexpected.txt", extra.stderr)
        os.remove(os.path.join(assets, "unexpected.txt"))
        with open(os.path.join(assets, f"fjx-{version}-x86_64-unknown-linux-gnu.tar.gz"), "ab") as file:
            file.write(b"bad")
        self.assertFailed(self.package(["verify", assets, version, *epoch]), "checksum failed")

    def verify_environment(self, bin_dir: str, log: str) -> dict:
        return {"PATH": with_path(bin_dir), "FAKE_RUNTIME_LOG": log}

    def formatted_assets(self, epoch: int = EPOCH) -> str:
        bins = seed_binaries(self.mkdir("format-bins"), formats=True, epoch=epoch)
        assets = self.path("formatted")
        self.assertSucceeded(self.package([assets, "--source-date-epoch", str(EPOCH)], {"FJX_PACKAGE_BIN_ROOT": bins}))
        return assets

    def test_verification_checks_formats_modes_and_pe_timestamp(self):
        bin_dir = self.mkdir("bin")
        install_tool(bin_dir, "file", "release_test_tools", "fake_file")
        log = self.path("runtime.jsonl")
        environment = self.verify_environment(bin_dir, log)
        version = repository_version()
        assets = self.formatted_assets()
        self.assertSucceeded(self.package(["verify", assets, version, "--source-date-epoch", str(EPOCH)], environment))
        self.assertFalse(os.path.exists(log), "verification without --smoke ran a container")
        mismatch = self.package(["verify", assets, version, "--source-date-epoch", "1795738652"], environment)
        self.assertFailed(mismatch, f"PE timestamp {EPOCH} of x86_64-pc-windows-gnu does not match the source date "
                                    "epoch 1795738652")
        shutil.rmtree(assets)
        bins = seed_binaries(self.path("format-bins"), formats=True)
        os.chmod(os.path.join(bins, "x86_64-unknown-linux-gnu", "fjx"), 0o644)
        # package_release always stages 0755; build an archive by hand with a non-executable binary.
        archive = os.path.join(assets, f"fjx-{version}-x86_64-unknown-linux-gnu.tar.gz")
        self.assertSucceeded(self.package([assets, "--source-date-epoch", str(EPOCH)], {"FJX_PACKAGE_BIN_ROOT": bins}))
        stage = self.mkdir("stage")
        shutil.copy(os.path.join(bins, "x86_64-unknown-linux-gnu", "fjx"), os.path.join(stage, "fjx"))
        os.chmod(os.path.join(stage, "fjx"), 0o644)
        subprocess.run(["tar", "-czf", archive, "-C", stage, "fjx"], check=True)
        with open(f"{archive}.sha256", "wb") as file:
            file.write(subprocess.run(["sha256sum", os.path.basename(archive)], cwd=assets, capture_output=True,
                                      check=True).stdout)
        self.assertFailed(self.package(["verify", assets, version, "--source-date-epoch", str(EPOCH)], environment),
                          "archive binary is not executable for x86_64-unknown-linux-gnu")

    def test_smoke_verification_uses_pinned_images(self):
        bin_dir = self.mkdir("bin")
        install_tool(bin_dir, "file", "release_test_tools", "fake_file")
        runtime = install_tool(bin_dir, "fake-runtime", "release_test_tools", "verify_runtime")
        wine = install_tool(bin_dir, "wine", "release_test_tools", "wine")
        log = self.path("runtime.jsonl")
        environment = {**self.verify_environment(bin_dir, log), "FORGEJO_TOKEN": "smoke-secret"}
        version = repository_version()
        assets = self.formatted_assets()
        result = self.package(["verify", assets, version, "--source-date-epoch", str(EPOCH), "--runtime", runtime,
                               "--wine", wine, "--smoke"], environment)
        self.assertSucceeded(result)
        calls = read_lines(log)
        self.assertTrue(all(call["secrets_in_env"] == [] for call in calls), "a credential reached a smoke tool")
        runs = [call for call in calls if call["args"][0] == "run"]
        self.assertEqual(len(runs), 2, "both Linux AMD64 targets need a container smoke")
        for call in runs:
            self.assertEqual(call["args"][:6], ["run", "--rm", "--interactive", "--platform", "linux/amd64",
                                                package_release.LINUX_AMD64_SMOKE_IMAGE])
            self.assertRegex(package_release.LINUX_AMD64_SMOKE_IMAGE, r"@sha256:[0-9a-f]{64}$")
        amd64_archives = sorted(os.path.join(assets, f"fjx-{version}-{triple}.tar.gz")
                                for triple in ("x86_64-unknown-linux-gnu", "x86_64-unknown-linux-musl"))
        import hashlib
        self.assertEqual(sorted(call["stdin_sha256"] for call in runs),
                         sorted(hashlib.sha256(read(path)).hexdigest() for path in amd64_archives))
        creates = [call["args"] for call in calls if call["args"][0] == "create"]
        self.assertEqual(creates.count(["create", "--platform", "linux/amd64", package_release.ARM64_QEMU_IMAGE]), 2)
        smoke_create = ["create", "--platform", "linux/arm64", "--entrypoint", "/usr/local/bin/qemu-aarch64",
                        package_release.LINUX_ARM64_SMOKE_IMAGE, "-L", "/", "/usr/local/bin/fjx", "/usr/local/bin/fjx",
                        "--help"]
        self.assertEqual(creates.count(smoke_create), 2)
        for image in (package_release.LINUX_ARM64_SMOKE_IMAGE, package_release.ARM64_QEMU_IMAGE):
            self.assertRegex(image, r"@sha256:[0-9a-f]{64}$")
        removed = [call["args"] for call in calls if call["args"][0] == "rm"]
        self.assertEqual(sorted(removed), sorted([["rm", "--force", "qemu-container"]] * 2 +
                                                 [["rm", "--force", "smoke-container"]] * 2))
        self.assertIn(["wine", "fjx.exe", "--help"], [call["args"] for call in calls])

        os.remove(log)
        failed = self.package(["verify", assets, version, "--source-date-epoch", str(EPOCH), "--runtime", runtime,
                               "--wine", wine, "--smoke"], {**environment, "FAKE_RUN_FAIL": "true"})
        self.assertFailed(failed)
        self.assertEqual(failed.returncode, 3, "a failed smoke command must keep its exit code")

    def test_build_runtime_command_shapes(self):
        log = self.path("runtime.log")
        version = repository_version()
        for name in ("podman", "docker"):
            runtime = install_tool(self.mkdir("bin"), name, "release_test_tools", "build_runtime")
            output = self.path(f"output-{name}")
            result = self.package(["build", output, "--runtime", runtime, "--source-date-epoch", str(EPOCH)],
                                  {"FAKE_BUILD_LOG": log})
            self.assertSucceeded(result)
            self.assertTrue(os.path.exists(os.path.join(output, f"fjx-{version}-x86_64-unknown-linux-gnu.tar.gz")))
        invocations = read_lines(log)
        self.assertEqual(len(invocations), 12, "fake runtimes did not build all six targets")
        for invocation in invocations[:6]:
            self.assertEqual(invocation[0], "build", "Podman did not use its native build command")
            self.assertNotIn("buildx", invocation)
        for invocation in invocations[6:]:
            self.assertEqual(invocation[:2], ["buildx", "build"], "Docker did not use Buildx")
        self.assertFailed(self.package(["build", self.path("no-epoch"), "--runtime", runtime]),
                          "--source-date-epoch")

    def test_build_retry_and_arguments(self):
        log = self.path("runtime.jsonl")
        runtime = install_tool(self.mkdir("bin"), "argument-logger", "release_test_tools", "build_runtime")
        result = self.package(["build", self.path("assets"), "--runtime", runtime, "--source-date-epoch", "1"],
                              {"FAKE_BUILD_LOG": log, "FAKE_BUILD_FAIL_TARGET": "x86_64-pc-windows-gnu"})
        self.assertFailed(result, "cross build failed twice for x86_64-pc-windows-gnu")
        self.assertIn("cross build failed for x86_64-pc-windows-gnu; retrying once", result.stderr)
        calls = read_lines(log)
        self.assertEqual(len(calls), 7, "the failed final target was not retried exactly once")
        self.assertTrue(all(call[:2] == ["buildx", "build"] for call in calls))
        built = [next(arg for arg in call if arg.startswith("TARGET="))[len("TARGET="):] for call in calls]
        self.assertEqual(built[:-1], [target["triple"] for target in targets()], "build order diverges from manifest")
        self.assertEqual(built[-2:], ["x86_64-pc-windows-gnu"] * 2, "build retried the wrong target")
        self.assertTrue(all(call[-1] == ROOT for call in calls), "project context did not reach every build")
        self.assertTrue(all("SOURCE_DATE_EPOCH=1" in call for call in calls), "epoch did not reach every build")
        self.assertTrue(all(["--file", RELEASE_CONTAINERFILE] == call[2:4] for call in calls))

    def test_smoke_requires_wine(self):
        assets = self.mkdir("assets")
        missing_wine = self.package(["verify", assets, repository_version(), "--source-date-epoch", str(EPOCH),
                                     "--smoke"])
        self.assertFailed(missing_wine, "--wine is required")
        self.assertNotIn("release asset inventory mismatch", missing_wine.stderr)

    def test_pe_timestamp(self):
        path = self.path("fjx.exe")
        write(path, pe_header(1795738652))
        self.assertEqual(package_release.pe_timestamp(path), 1795738652)
        write(path, pe_header(0xFFFFFFFF))
        self.assertEqual(package_release.pe_timestamp(path), 0xFFFFFFFF, "timestamp must be unsigned")
        for data, message in ((b"MZ\0\0", "no MZ header"), (b"#!/bin/sh\n" + bytes(80), "no MZ header"),
                              (b"MZ" + bytes(58) + (1000).to_bytes(4, "little") + bytes(20), "no PE header")):
            write(path, data)
            with self.assertRaisesRegex(release.Failure, message):
                package_release.pe_timestamp(path)

    def test_flatten_and_matrix(self):
        platform = self.mkdir("platform")
        write(os.path.join(platform, "linux", "a.tar.gz"), "a")
        write(os.path.join(platform, "windows", "nested", "b.zip"), "b")
        assets = self.path("assets")
        self.assertSucceeded(self.package(["flatten", platform, assets]))
        self.assertEqual(sorted(os.listdir(assets)), ["a.tar.gz", "b.zip"])
        self.assertFailed(self.package(["flatten", platform, assets]), "asset directory already exists")
        write(os.path.join(platform, "other", "a.tar.gz"), "c")
        self.assertFailed(self.package(["flatten", platform, self.path("again")]), "duplicate flattened asset a.tar.gz")
        matrix = self.package(["matrix"])
        self.assertSucceeded(matrix)
        self.assertEqual(json.loads(matrix.stdout), targets())

    def test_release_build_contract(self):
        manifest = targets()
        self.assertEqual([target["triple"] for target in manifest],
                         ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu", "x86_64-unknown-linux-musl",
                          "aarch64-unknown-linux-musl", "aarch64-apple-darwin", "x86_64-pc-windows-gnu"])
        self.assertEqual([target["runtime"] for target in manifest if target["os"] == "macos"], ["none"])
        windows = next(target for target in manifest if target["os"] == "windows")
        self.assertEqual((windows["archive"], windows["executable"], windows["runtime"]), ("zip", "fjx.exe", "wine"))
        for target in manifest:
            if not target["triple"].endswith("musl"):
                continue
            arch = "x86-64" if target["arch"] == "x86_64" else "ARM aarch64"
            self.assertRegex(f"ELF 64-bit LSB executable, {arch}, statically linked", target["file_pattern"])
            self.assertRegex(f"ELF 64-bit LSB pie executable, {arch}, static-pie linked", target["file_pattern"])
            self.assertNotRegex(f"ELF 64-bit LSB pie executable, {arch}, dynamically linked", target["file_pattern"])
        self.assertRegex("PE32+ executable for MS Windows 6.00 (console), x86-64, 6 sections", windows["file_pattern"])
        self.assertNotRegex("PE32 executable for MS Windows 6.00 (console), Intel 80386, 5 sections",
                            windows["file_pattern"])
        containerfile = read(RELEASE_CONTAINERFILE).decode()
        for required in [
            "ghcr.io/rust-cross/cargo-zigbuild:0.23.3@sha256:"
            "76ed3823d8cd9d8b409b10f9c4cda292b0c8699175ea4c0a2d541775c8184d2b",
            'test "$(/usr/local/cargo/bin/cargo-zigbuild --version)" = "cargo-zigbuild 0.23.3"',
            'test "$(zig version)" = 0.16.0',
            'test "$SDKROOT" = /opt/MacOSX11.3.sdk',
            "for attempt in 1 2 3 4 5; do",
            "rustup toolchain install 1.97.1 --profile minimal && break;",
            'if [ "$attempt" = 5 ]; then exit 1; fi;',
            'sleep "$attempt";',
            'cargo zigbuild --locked --release --target "$TARGET"',
            "ARG SOURCE_DATE_EPOCH",
        ]:
            self.assertIn(required, containerfile)
        for forbidden in ("cargo install --list", "cargo zigbuild --version", ":latest"):
            self.assertNotIn(forbidden, containerfile)

    def test_manifest_shape_is_strict(self):
        original = read(RELEASE_TARGETS).decode()
        cases = [
            (original.replace("schema = 1", "schema = true"), "unsupported target manifest schema"),
            (original.replace("schema = 1", "schema = 2"), "unsupported target manifest schema 2"),
            (original.replace('runtime = "none"', 'runtime = "linux/amd64"'), "macOS targets cannot claim"),
            (original.replace('archive = "zip"', 'archive = "tar.gz"'), "Windows must ship fjx.exe in a zip"),
            (original.replace('triple = "aarch64-apple-darwin"', 'triple = "x86_64-apple-darwin"'), "x86_64 macOS"),
            (original.replace('triple = "aarch64-apple-darwin"', 'triple = "x86_64-unknown-linux-gnu"'), "duplicate"),
            (original.replace('runtime = "wine"', 'runtime = 1'), "target manifest entries must name"),
        ]
        for text, message in cases:
            with self.subTest(message=message):
                path = self.path("release-targets.toml")
                write(path, text)
                with mock.patch.object(package_release, "MANIFEST_PATH", path), \
                        self.assertRaisesRegex(release.Failure, message):
                    package_release.release_targets()


class SmokeStateMachineTests(TempTestCase):
    def exercise(self, states: list[str], timeout: float = 2, poll_interval: float = 0.001, fail_copy: str = ""):
        root = self.mkdir(f"smoke-{len(os.listdir(self.tmp))}")
        runtime = install_tool(root, "fake-smoke-runtime", "release_test_tools", "smoke_runtime")
        calls = os.path.join(root, "calls.jsonl")
        write(os.path.join(root, "states.txt"), "\n".join(states))
        write(os.path.join(root, "index.txt"), "0")
        write(calls, "")
        environment = {"FAKE_SMOKE_CALLS": calls, "FAKE_SMOKE_STATES": os.path.join(root, "states.txt"),
                       "FAKE_SMOKE_INDEX": os.path.join(root, "index.txt"), "FAKE_SMOKE_FAIL_COPY": fail_copy}
        error = ""
        with mock.patch.dict(os.environ, environment):
            try:
                release_smoke.run_arm64_smoke_container(runtime, "fake-smoke-container", "linux/arm64",
                                                        "/fake/qemu-aarch64", "/fake/fjx", timeout, poll_interval)
            except release.Failure as failure:
                error = str(failure)
        return error, read_lines(calls)

    def assert_cleanup(self, calls: list):
        self.assertEqual(calls[-1], {"command": "rm", "arguments": ["--force", "fake-smoke-container"]})

    def test_arm64_smoke_state_machine(self):
        error, calls = self.exercise(['[{"State":{"Status":"running","ExitCode":0}}]',
                                      '[{"State":{"Status":"exited","ExitCode":0}}]'])
        self.assertEqual(error, "")
        self.assertEqual([call["command"] for call in calls], ["cp", "cp", "start", "inspect", "inspect", "logs", "rm"])
        self.assertEqual(calls[0]["arguments"], ["/fake/qemu-aarch64", "fake-smoke-container:/usr/local/bin/qemu-aarch64"])
        self.assertEqual(calls[1]["arguments"], ["/fake/fjx", "fake-smoke-container:/usr/local/bin/fjx"])
        self.assert_cleanup(calls)

        error, calls = self.exercise(['[{"State":{"Status":"exited","ExitCode":7}}]'])
        self.assertIn("exit code 7", error)
        self.assertIn("logs: fake-fjx-help", error)
        self.assertEqual([call["command"] for call in calls], ["cp", "cp", "start", "inspect", "logs", "rm"])
        self.assert_cleanup(calls)

        error, calls = self.exercise(['[{"State":{"Status":"exited","ExitCode":0}}]'], fail_copy="fjx")
        self.assertIn("could not prepare", error)
        self.assertEqual([call["command"] for call in calls], ["cp", "cp", "rm"])
        self.assert_cleanup(calls)

        for state, message in (("{", "invalid ARM64 Linux smoke state"),
                               ('{"State":{"Status":"exited"}}', "invalid ARM64 Linux smoke state"),
                               ("[]", "unexpected ARM64 Linux smoke state count"),
                               ('[{"State":{"Status":"exited","ExitCode":"x"}}]', "nonnumeric"),
                               ('[{"State":{"Status":"dead","ExitCode":0}}]', "with status dead"),
                               ('[{"State":{"Status":"paused","ExitCode":0}}]', "unexpected status paused")):
            with self.subTest(state=state):
                error, calls = self.exercise([state])
                self.assertIn(message, error)
                self.assert_cleanup(calls)

        error, calls = self.exercise(['[{"State":{"Status":"running","ExitCode":0}}]'], 0.002, 0.005)
        self.assertIn("timed out after 2ms", error)
        self.assertIn("logs", [call["command"] for call in calls])
        self.assert_cleanup(calls)

        for timeout, interval in ((0, 1), (1, 0)):
            with self.assertRaisesRegex(release.Failure, "must be positive"):
                release_smoke.run_arm64_smoke_container("unused", "c", "linux/arm64", "q", "b", timeout, interval)

    def test_duration_format(self):
        self.assertEqual([release_smoke.format_duration(value) for value in (30, 0.25, 0.002, 90, 1.5)],
                         ["30sec", "250ms", "2ms", "1min 30sec", "1sec 500ms"])


class PublicationTests(TempTestCase):
    """Publication commands against small fakes for git and curl."""

    def fake_bin(self, curl_function: str) -> str:
        bin_dir = self.mkdir("bin")
        install_tool(bin_dir, "git", "release_test_tools", "scoped_git")
        install_tool(bin_dir, "curl", "release_test_tools", curl_function)
        return bin_dir

    def test_publish_planning(self):
        make_fixture(self.tmp)
        with mock.patch.dict(os.environ, {"RELEASE_WORKSPACE_ROOT": self.tmp}):
            absent = release.publication_plan("abc", "abc", 2, "123")
            self.assertEqual(absent, {"publish": True, "tag": "fjx/v1.2.3", "version": "1.2.3",
                                      "source_date_epoch": "123"})
            self.assertFalse(release.publication_plan("abc", "abc", 0, "123")["publish"])
            with self.assertRaisesRegex(release.Failure, "does not match"):
                release.publication_plan("abc", "def", 2, "123")
            with self.assertRaisesRegex(release.Failure, "git ls-remote exited 1"):
                release.publication_plan("abc", "abc", 1, "123")

    def metadata_env(self, bin_dir: str, state: str, **extra) -> dict:
        return {"PATH": with_path(bin_dir), "FORGEJO_API_URL": "https://forge.example/api/v1",
                "FORGEJO_REPOSITORY": "dalton/monolith", "FORGEJO_TOKEN": "fixture", "FAKE_REVISION": REVISION,
                "FAKE_METADATA": state, "FAKE_ASSET_NAMES": json.dumps(expected_asset_names("1.2.3")),
                "FAKE_EXPECTED_TOKEN": "fixture", **extra}

    def test_release_metadata_mutations(self):
        make_fixture(self.tmp)
        bin_dir = self.fake_bin("metadata_curl")
        for state, message in (("title", "metadata is not exact"), ("body", "metadata is not exact"),
                               ("prerelease", "metadata is not exact"), ("tag", "invalid release identity"),
                               ("draft-number", "has no draft state")):
            with self.subTest(state=state):
                output = write(self.path(f"metadata-{state}"), "")
                result = run_release(self.tmp, ["publish-plan", REVISION, "123", output],
                                     self.metadata_env(bin_dir, state))
                self.assertFailed(result, message)
                self.assertEqual(read(output), b"")

    def test_publish_plan_output_and_scoped_git_authentication(self):
        make_fixture(self.tmp)
        bin_dir = self.fake_bin("metadata_curl")
        output = write(self.path("plan"), "existing=1\n")
        git_log, curl_log = self.path("git.jsonl"), self.path("curl.jsonl")
        environment = self.metadata_env(bin_dir, "valid", FAKE_GIT_LOG=git_log, FAKE_CURL_LOG=curl_log)
        result = run_release(self.tmp, ["publish-plan", REVISION, "123", output], environment)
        self.assertSucceeded(result)
        self.assertEqual(result.stdout, "")
        self.assertEqual(read(output).decode(), "existing=1\npublish=true\nneeds_tag=false\nneeds_release=false\n"
                         "needs_assets=true\nneeds_publication=false\nreplace_assets=false\nmissing_assets=\n"
                         "release_id=42\ntag=fjx/v1.2.3\nversion=1.2.3\nsource_date_epoch=123\n")
        calls = read_lines(git_log)
        self.assertEqual([call["args"] for call in calls],
                         [["rev-parse", "HEAD"], ["remote", "get-url", "origin"],
                          ["ls-remote", "--tags", "origin", "refs/tags/fjx/v*"]])
        ls_remote = calls[-1]
        self.assertEqual(ls_remote["count"], "2")
        self.assertEqual(ls_remote["key_0"], "http.https://forge.example/.extraHeader")
        self.assertTrue(ls_remote["value_0_ok"], "Git did not get the exact Basic credential")
        self.assertEqual((ls_remote["key_1"], ls_remote["value_1"]), ("http.followRedirects", "false"))
        self.assertTrue(all(call["secrets_in_env"] == [] for call in calls), "a credential reached git's environment")
        self.assertIsNone(calls[0]["key_0"], "unauthenticated git calls must not get the credential")
        curl_calls = read_lines(curl_log)
        self.assertEqual(len(curl_calls), 1)
        self.assertEqual(curl_calls[0]["args"][-1], "https://forge.example/api/v1/repos/dalton/monolith/releases/tags/fjx%2Fv1.2.3")
        self.assertTrue(curl_calls[0]["stdin_ok"], "curl did not get the token header on standard input")
        self.assertEqual(curl_calls[0]["secrets_in_env"], [])
        self.assertNotIn("fixture", json.dumps(curl_calls[0]["args"]))

        for override, message in (({"FAKE_GIT_URL": "https://evil.example/dalton/monolith.git"},
                                   "Git origin must be the exact HTTPS Forgejo repository"),
                                  ({"FORGEJO_API_URL": "http://forge.example/api/v1"},
                                   "Git origin must use the exact HTTPS Forgejo origin"),
                                  ({"FORGEJO_API_URL": "https://forge.example/v1"}, "must end with /api/v1"),
                                  ({"FORGEJO_REPOSITORY": "dalton/mono lith"}, "repository are required")):
            with self.subTest(override=override):
                os.remove(git_log)
                result = run_release(self.tmp, ["publish-plan", REVISION, "123", output], {**environment, **override})
                self.assertFailed(result, message)
                self.assertNotIn("ls-remote", [call["args"][0] for call in read_lines(git_log)])

    def test_prior_released_versions_must_be_lower(self):
        make_fixture(self.tmp)
        bin_dir = self.fake_bin("metadata_curl")
        output = write(self.path("plan"), "")
        higher = self.metadata_env(bin_dir, "valid", FAKE_EXTRA_TAGS=f"{'c' * 40} refs/tags/fjx/v1.10.0^{{}}")
        self.assertFailed(run_release(self.tmp, ["publish-plan", REVISION, "123", output], higher),
                          "release version 1.2.3 must be greater than prior released version 1.10.0")
        lower = self.metadata_env(bin_dir, "valid", FAKE_EXTRA_TAGS=f"{'c' * 40} refs/tags/fjx/v1.2.2\n"
                                                                   f"{'d' * 40} refs/tags/fjx/v01.9.9")
        self.assertSucceeded(run_release(self.tmp, ["publish-plan", REVISION, "123", output], lower))
        moved = self.metadata_env(bin_dir, "valid", FAKE_TAG_TARGET="e" * 40)
        self.assertFailed(run_release(self.tmp, ["publish-plan", REVISION, "123", output], moved),
                          f"fjx/v1.2.3 targets {'e' * 40} instead of {REVISION}; refusing to move it")

    def test_authenticated_release_origin(self):
        project = make_fixture(self.tmp)
        assets, payload = self.mkdir("fjx", "publish-assets"), self.mkdir("payload")
        bin_dir = self.fake_bin("origin_curl")
        curl_log, foreign_log = self.path("curl.log"), self.path("foreign.log")
        write(os.path.join(payload, "fjx"), "#!/bin/sh\necho fixture\n", 0o755)
        write(os.path.join(payload, "fjx.exe"), "#!/bin/sh\necho fixture\n", 0o755)
        for target in targets():
            archive = f"fjx-1.2.3-{target['triple']}.{target['archive']}"
            if target["archive"] == "tar.gz":
                subprocess.run(["tar", "-czf", os.path.join(assets, archive), "-C", payload, target["executable"]],
                               check=True)
            else:
                subprocess.run(["zip", "-X", "-q", os.path.join(assets, archive), target["executable"]], cwd=payload,
                               check=True)
            write(os.path.join(assets, f"{archive}.sha256"),
                  subprocess.run(["sha256sum", archive], cwd=assets, capture_output=True, check=True).stdout)
        environment = {"PATH": with_path(bin_dir), "FORGEJO_API_URL": "https://forge.example/api/v1",
                       "FORGEJO_REPOSITORY": "dalton/monolith", "FORGEJO_TOKEN": "origin-fixture-secret",
                       "FAKE_REVISION": REVISION, "FAKE_ASSET_ROOT": assets, "FAKE_CURL_LOG": curl_log,
                       "FAKE_FOREIGN_LOG": foreign_log, "FAKE_ASSET_NAMES": json.dumps(expected_asset_names("1.2.3")),
                       "FAKE_EXPECTED_TOKEN": "origin-fixture-secret"}
        arguments = ["verify-final", "1.2.3", REVISION, "fjx/v1.2.3"]

        valid = run_release(self.tmp, arguments, {**environment, "FAKE_RELEASE_STATE": "valid"})
        self.assertSucceeded(valid)
        calls = read_lines(curl_log)
        self.assertEqual(len(calls), 13)
        downloads = [call for call in calls if "/releases/download/" in call["args"][-1]]
        self.assertEqual(sorted(call["args"][-1] for call in downloads),
                         sorted(f"https://forge.example/dalton/monolith/releases/download/fjx%2Fv1.2.3/{name}"
                                for name in expected_asset_names("1.2.3")))
        for call in calls:
            arguments_text = json.dumps(call["args"])
            self.assertNotIn("/releases/assets/", arguments_text)
            self.assertNotIn("--location", call["args"])
            self.assertNotIn("origin-fixture-secret", arguments_text)
            self.assertTrue(call["stdin_ok"], "every request must read its token header from standard input")
            self.assertEqual(call["secrets_in_env"], [])
            joined = " ".join(call["args"])
            self.assertIn("--proto =https --max-redirs 0 --config -", joined)
        self.assertTrue(all("--header Accept: application/octet-stream" in " ".join(call["args"]) for call in downloads))

        subpath = run_release(self.tmp, arguments, {**environment, "FORGEJO_API_URL": "https://forge.example/forgejo/api/v1",
                                                    "FAKE_GIT_URL": "https://forge.example/forgejo/dalton/monolith.git",
                                                    "FAKE_RELEASE_STATE": "valid"})
        self.assertSucceeded(subpath)

        for state, message in (("foreign", "must use the exact HTTPS Forgejo API origin"),
                               ("foreign-path", "must identify this exact repository, tag, and asset"),
                               ("http-asset", "must use the exact HTTPS Forgejo API origin"),
                               ("redirect", "could not download release asset"),
                               ("missing-id", "has no valid Forgejo asset ID"),
                               ("invalid-id", "has no valid Forgejo asset ID"),
                               ("bool-id", "has no valid Forgejo asset ID"),
                               ("corrupt", "does not match the locally checked output")):
            with self.subTest(state=state):
                write(curl_log, "")
                write(foreign_log, "")
                result = run_release(self.tmp, arguments, {**environment, "FAKE_RELEASE_STATE": state})
                self.assertFailed(result, message)
                self.assertNotIn("origin-fixture-secret", result.stderr + result.stdout)
                unsafe = read(curl_log).decode()
                self.assertNotIn("https://evil.example", unsafe)
                self.assertNotIn("http://forge.example", unsafe)
                self.assertEqual(read(foreign_log), b"")
                if state == "redirect":
                    self.assertIn("/releases/download/fjx%2Fv1.2.3/", unsafe)

        for api in ("http://forge.example/api/v1", "https://user@forge.example/api/v1",
                    "https://forge.example:bad/api/v1", "forge.example/api/v1"):
            with self.subTest(api=api):
                write(curl_log, "")
                result = run_release(self.tmp, arguments, {**environment, "FORGEJO_API_URL": api,
                                                           "FAKE_RELEASE_STATE": "valid"})
                self.assertFailed(result)
                self.assertEqual(read(curl_log), b"", f"authenticated request started for {api}")
                self.assertNotIn("origin-fixture-secret", result.stderr)
        os.remove(os.path.join(project, "publish-assets", expected_asset_names("1.2.3")[0]))
        self.assertFailed(run_release(self.tmp, arguments, {**environment, "FAKE_RELEASE_STATE": "valid"}),
                          "locally checked release assets are unavailable or not exact")

    def test_create_release(self):
        bin_dir = self.fake_bin("create_curl")
        calls = self.path("calls")
        environment = {"PATH": with_path(bin_dir), "FORGEJO_API_URL": "https://forge.example/forgejo/api/v1",
                       "FORGEJO_REPOSITORY": "dalton/fjx", "FORGEJO_TOKEN": "fixture-create",
                       "FAKE_CREATE_CALLS": calls, "FAKE_CREATE_STATUS": "201"}
        with mock.patch.dict(os.environ, environment):
            self.assertEqual(release.create_release("1.2.3", "fjx/v1.2.3"), 42)
            write(calls, "")
            with mock.patch.dict(os.environ, {"FAKE_CREATE_STATUS": "503"}), \
                    self.assertRaisesRegex(release.Failure, r"refused release creation \(HTTP 503\)"):
                release.create_release("1.2.3", "fjx/v1.2.3")
            self.assertEqual(read(calls), b"request\n", "release creation must never retry a write")
            with mock.patch.dict(os.environ, {"FORGEJO_TOKEN": " "}), \
                    self.assertRaisesRegex(release.Failure, "token are required"):
                release.create_release("1.2.3", "fjx/v1.2.3")

    def test_errors_are_plain_and_redacted(self):
        with mock.patch.dict(os.environ, {"FORGEJO_TOKEN": "very-secret-value"}), \
                mock.patch("sys.stderr") as stderr:
            def explode(_):
                raise RuntimeError("boom very-secret-value")
            self.assertEqual(release.cli_main(explode, []), 1)
        written = "".join(call.args[0] for call in stderr.write.call_args_list)
        self.assertEqual(written, "error: unexpected RuntimeError: boom [redacted]\n")

    def test_scripts_use_argument_lists(self):
        for name in ("release.py", "package_release.py", "release_smoke.py"):
            source = read(os.path.join(SCRIPTS, name)).decode()
            self.assertNotIn("shell=True", source, name)
            self.assertNotIn("os.system", source, name)
            self.assertNotIn("--location", source, name)


class PublicationFlowTests(TempTestCase):
    """The publication commands against a stateful fake Forgejo (fixtures/forgejo_release_tools.py)."""

    VERSION = "1.2.10"
    TAG = "fjx/v1.2.10"

    def setUp(self):
        super().setUp()
        real_git = real_program("git")
        self.project = make_fixture(self.tmp, self.VERSION)
        self.state = self.mkdir("server")
        self.mkdir("server", "assets")
        write(os.path.join(self.state, "calls"), "")
        bin_dir = self.mkdir("bin")
        for name in ("git", "curl"):
            install_tool(bin_dir, name, "forgejo_release_tools", name)
        self.git(real_git, "init", "--quiet")
        self.git(real_git, "add", ".")
        self.git(real_git, "-c", "user.name=Fixture", "-c", "user.email=fixture@localhost", "commit", "--quiet",
                 "-m", self.VERSION)
        self.revision = self.git(real_git, "rev-parse", "HEAD")
        self.real_git = real_git
        bins = seed_binaries(self.mkdir("bins"))
        self.assertSucceeded(run_script("package_release.py",
                                        [os.path.join(self.project, "publish-assets"), "--source-date-epoch", "1"],
                                        {"FJX_PACKAGE_BIN_ROOT": bins, "RELEASE_WORKSPACE_ROOT": self.tmp}))
        self.env = {"PATH": with_path(bin_dir), "RELEASE_WORKSPACE_ROOT": self.tmp, "FAKE_STATE": self.state,
                    "FAKE_REAL_GIT": real_git, "FORGEJO_API_URL": "https://forge.example/forgejo/api/v1",
                    "FORGEJO_REPOSITORY": "dalton/fjx", "FORGEJO_TOKEN": "fixture", "FAKE_STRICT_CURL": "true",
                    "FAKE_EXPECTED_TOKEN": "fixture"}

    def git(self, *arguments: str) -> str:
        return subprocess.run(list(arguments), cwd=self.project, capture_output=True, text=True,
                              check=True).stdout.strip()

    def release(self, *arguments: str, **extra: str):
        return run_script("release.py", list(arguments), {**self.env, **extra}, cwd=self.project)

    def plan(self, **extra: str) -> dict:
        output = self.path("plan")
        write(output, "")
        self.assertSucceeded(self.release("publish-plan", self.revision, "1", output, **extra))
        return dict(line.split("=", 1) for line in read(output).decode().splitlines())

    def create(self) -> int:
        with mock.patch.dict(os.environ, base_env(self.env), clear=True):
            return release.create_release(self.VERSION, self.TAG)

    def tag(self):
        self.git(self.real_git, "-c", "user.name=Forgejo Actions", "-c", "user.email=actions@localhost", "tag",
                 "--annotate", self.TAG, "--message", f"fjx v{self.VERSION}", self.revision)

    def events(self) -> list[tuple[str, str]]:
        return [(call["event"], call["value"]) for call in read_lines(os.path.join(self.state, "calls"))]

    def release_record(self) -> dict:
        return json.loads(read(os.path.join(self.state, "release.json")))

    def publish_new_draft(self) -> dict:
        plan = self.plan()
        self.assertEqual(self.create(), 42)
        self.assertSucceeded(self.release("upload-missing-assets", "42", plan["missing_assets"]))
        return plan

    def test_first_publication_pushes_the_tag_last(self):
        plan = self.plan()
        names = expected_asset_names(self.VERSION)
        self.assertEqual(plan, {"publish": "true", "needs_tag": "true", "needs_release": "true", "needs_assets": "true",
                                "needs_publication": "true", "replace_assets": "false",
                                "missing_assets": ",".join(sorted(names)), "release_id": "", "tag": self.TAG,
                                "version": self.VERSION, "source_date_epoch": "1"})
        self.assertEqual(self.create(), 42)
        self.assertSucceeded(self.release("upload-missing-assets", "42", plan["missing_assets"]))
        self.assertEqual(len(self.release_record()["assets"]), 12)
        self.tag()
        self.assertSucceeded(self.release("publish-release", self.VERSION, self.revision, self.TAG))
        self.assertSucceeded(self.release("verify-final", self.VERSION, self.revision, self.TAG))
        events = self.events()
        uploads = [index for index, event in enumerate(events) if event[0] == "POST" and "?name=" in event[1]]
        push = events.index(("push-tag", ""))
        patch = next(index for index, event in enumerate(events) if event[0] == "PATCH")
        self.assertEqual(len(uploads), 12)
        self.assertLess(max(uploads), push, "the tag must be pushed after every asset is uploaded")
        self.assertLess(push, patch, "the tag must be pushed right before publication")
        self.assertIn(("public", "12"), events, "the release became public before all assets were present")
        self.assertNotIn("DELETE", [event[0] for event in events])
        self.assertEqual(len([event for event in events if event[0] == "download"]), 12)
        self.assertFalse(self.release_record()["draft"])

        done = self.plan()
        self.assertEqual((done["needs_tag"], done["needs_release"], done["needs_publication"], done["missing_assets"],
                          done["release_id"]), ("false", "false", "false", "", "42"))

    def test_rerun_replaces_untagged_draft_assets(self):
        self.publish_new_draft()
        plan = self.plan()
        self.assertEqual((plan["needs_release"], plan["replace_assets"], plan["release_id"]), ("false", "true", "42"))
        self.assertEqual(plan["missing_assets"], ",".join(sorted(expected_asset_names(self.VERSION))))
        # The fake server, like Forgejo, never lets an upload replace an asset.
        self.assertFailed(self.release("upload-missing-assets", "42", plan["missing_assets"]),
                          "Forgejo API request failed")
        self.assertSucceeded(self.release("delete-draft-assets", "42", self.revision))
        self.assertEqual(len([event for event in self.events() if event[0] == "DELETE"]), 12)
        self.assertEqual(self.release_record()["assets"], [])
        self.assertSucceeded(self.release("upload-missing-assets", "42", plan["missing_assets"]))
        self.assertEqual(sorted(asset["id"] for asset in self.release_record()["assets"]), list(range(13, 25)))
        self.tag()
        self.assertSucceeded(self.release("publish-release", self.VERSION, self.revision, self.TAG))
        self.assertSucceeded(self.release("verify-final", self.VERSION, self.revision, self.TAG))
        self.assertFailed(self.release("delete-draft-assets", "43", self.revision), "already pushed")

    def test_pushed_tag_stops_draft_deletes(self):
        self.publish_new_draft()
        self.tag()
        self.assertSucceeded(self.release("push-tag", self.TAG, self.revision, FAKE_TAG_SYNC="never"))
        self.assertTrue(self.release_record()["draft"])
        plan = self.plan()
        self.assertEqual((plan["needs_tag"], plan["replace_assets"], plan["missing_assets"]), ("false", "false", ""))
        self.assertFailed(self.release("delete-draft-assets", "42", self.revision),
                          f"{self.TAG} is already pushed; refusing to delete release assets")
        self.assertSucceeded(self.release("publish-release", self.VERSION, self.revision, self.TAG))
        self.assertEqual(self.events().count(("push-tag", "")), 1, "publish-release pushed an existing tag again")

    def test_draft_metadata_is_checked_before_the_tag_push(self):
        self.publish_new_draft()
        self.tag()
        for mode, message in (("size", "size does not match the locally checked output"),
                              ("origin", "must use the exact HTTPS Forgejo API origin"),
                              ("id", "has no valid Forgejo asset ID")):
            with self.subTest(mode=mode):
                self.assertFailed(self.release("publish-release", self.VERSION, self.revision, self.TAG,
                                               FAKE_DRAFT_METADATA=mode), message)
                self.assertNotIn(("push-tag", ""), self.events())
                self.assertTrue(self.release_record()["draft"])

    def test_failed_publication_and_byte_mismatch(self):
        self.publish_new_draft()
        self.tag()
        self.assertFailed(self.release("publish-release", self.VERSION, self.revision, self.TAG,
                                       FAKE_AMBIGUOUS_PUBLISH="true"), "Forgejo refused publication (HTTP 503)")
        self.assertFailed(self.release("verify-final", self.VERSION, self.revision, self.TAG,
                                       FAKE_CORRUPT_DOWNLOAD="true"), "does not match the locally checked output")
        self.assertSucceeded(self.release("verify-final", self.VERSION, self.revision, self.TAG))
        self.assertFailed(self.release("verify-final", "1.2.9", self.revision, "fjx/v1.2.9"),
                          "final release identity does not match project version")
        self.assertFailed(self.release("publish-release", self.VERSION, self.revision, self.TAG),
                          "metadata is not exact")

    def test_push_tag_guards(self):
        message = "refusing to push a missing, lightweight, or wrongly targeted release tag"
        self.assertFailed(self.release("push-tag", self.TAG, self.revision), message)
        self.git(self.real_git, "tag", self.TAG, self.revision)
        self.assertFailed(self.release("push-tag", self.TAG, self.revision), message)
        self.git(self.real_git, "tag", "--delete", self.TAG)
        self.tag()
        self.assertFailed(self.release("push-tag", self.TAG, "b" * 40), message)
        self.assertFailed(self.release("push-tag", self.TAG, self.revision, FORGEJO_TOKEN=" "),
                          "Forgejo token is required to push a release tag")
        self.assertNotIn(("push-tag", ""), self.events())
        self.assertSucceeded(self.release("push-tag", self.TAG, self.revision))
        self.assertEqual(self.events(), [("push-tag", "")])

    def test_upload_guards(self):
        self.assertEqual(self.create(), 42)
        name = expected_asset_names(self.VERSION)[0]
        self.assertFailed(self.release("upload-missing-assets", "42", "evil.txt"), "refusing unexpected attachment evil.txt")
        self.assertFailed(self.release("upload-missing-assets", "42", f"{name}/../{name}"), "refusing unexpected")
        self.assertFailed(self.release("upload-missing-assets", "42", name, FAKE_FAIL_UPLOAD=name),
                          f"Forgejo refused missing {name} attachment (HTTP 503)")
        os.remove(os.path.join(self.project, "publish-assets", name))
        self.assertFailed(self.release("upload-missing-assets", "42", name), f"missing prepared attachment {name}")
        self.assertFailed(self.release("upload-missing-assets", "4x2", name), "invalid integer")
        self.assertEqual(self.release_record()["assets"], [])


if __name__ == "__main__":
    unittest.main()
