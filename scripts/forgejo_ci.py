#!/usr/bin/env python3
"""Forgejo CI entry points: pick a release candidate, build assets, publish."""

from __future__ import annotations

import os
import re
import shutil
import sys
import tempfile
import tomllib

from release import (ArgumentParser, Failure, checked, child_env, cli_main, complete, create_release,
                     publication_state, valid_version, validate_versions, version_greater)
from package_release import release_targets, verify_assets

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
PROJECT_DIR = os.path.dirname(SCRIPT_DIR)
RELEASE_SCRIPT = os.path.join(SCRIPT_DIR, "release.py")
PACKAGE_SCRIPT = os.path.join(SCRIPT_DIR, "package_release.py")
REVISION_PATTERN = re.compile(r"[0-9a-f]{40}([0-9a-f]{24})?")
NULL_REVISION_PATTERN = re.compile(r"0{40}(0{24})?")
USAGE = "usage: scripts/forgejo_ci.py candidate BEFORE SHA OUTPUT | build SHA | publish SHA"


def fail(message: str):
    raise Failure(f"fjx Forgejo CI: {message}")


def step(arguments: list[str], label: str, env: dict[str, str] | None = None) -> None:
    """Runs one publication step; its output goes to this job's log."""
    try:
        checked(arguments, env=env)
    except Failure:
        fail(f"{label} failed; publication stopped")


def release_step(*arguments: str) -> None:
    # child_env() drops every token. Only release.py talks to Forgejo, so it
    # alone gets FORGEJO_TOKEN back.
    extra = {"FORGEJO_TOKEN": os.environ["FORGEJO_TOKEN"]} if "FORGEJO_TOKEN" in os.environ else {}
    step([sys.executable, RELEASE_SCRIPT, *arguments], "release.py", env=child_env(extra))


def git_output(arguments: list[str]) -> str:
    result = complete(["git", *arguments])
    return result.stdout.strip() if result.exit_code == 0 else ""


def release_candidate(before: str, revision: str) -> bool:
    """A release needs a version increase across this push, not just a missing
    tag. Ordinary commits and initial imports must never publish an old version."""
    version = validate_versions()
    if not REVISION_PATTERN.fullmatch(revision) or git_output(["rev-parse", "HEAD"]) != revision:
        fail("checked-out revision does not match the push")
    if NULL_REVISION_PATTERN.fullmatch(before):
        return False
    if not REVISION_PATTERN.fullmatch(before):
        fail("push has no valid before revision")
    if complete(["git", "merge-base", "--is-ancestor", before, revision]).exit_code != 0:
        fail("before revision must be an ancestor of the checked commit")
    previous = complete(["git", "show", f"{before}:release.toml"])
    if previous.exit_code != 0:
        fail("could not read release.toml before this push")
    try:
        old_version = tomllib.loads(previous.stdout)["version"]
    except (tomllib.TOMLDecodeError, KeyError):
        fail("invalid previous release.toml")
    if not valid_version(old_version):
        fail("invalid previous version")
    if old_version == version:
        return False
    if not version_greater(version, old_version):
        fail("release version must increase")
    return True


def source_date_epoch(revision: str) -> str:
    epoch = git_output(["show", "-s", "--format=%ct", revision])
    if not re.fullmatch(r"[0-9]+", epoch) or int(epoch) <= 0:
        fail("release commit has no valid source timestamp")
    return epoch


def build_assets(revision: str) -> None:
    """Runs inside the pinned cargo-zigbuild image; no container socket is needed.

    No child gets a token: child_env() drops them, so Cargo build scripts never
    see one.
    """
    os.chdir(PROJECT_DIR)
    os.environ.pop("RELEASE_WORKSPACE_ROOT", None)
    version = validate_versions()
    if git_output(["rev-parse", "HEAD"]) != revision:
        fail("release checkout changed")
    epoch = source_date_epoch(revision)
    if os.path.lexists("publish-assets"):
        fail("publish-assets must not already exist")
    target_dir = os.environ.get("CARGO_TARGET_DIR") or os.path.join(PROJECT_DIR, "target")
    binaries = tempfile.mkdtemp()
    try:
        for target in release_targets():
            # The Windows linker writes the link time into the PE header unless
            # SOURCE_DATE_EPOCH is set. Pin it so a rerun builds the same bytes.
            step(["cargo", "zigbuild", "--locked", "--release", "--target", target["triple"]], "cargo",
                 env=child_env({"SOURCE_DATE_EPOCH": epoch}))
            directory = os.path.join(binaries, target["triple"])
            os.makedirs(directory)
            binary = os.path.join(target_dir, target["triple"], "release", target["executable"])
            shutil.copy(binary, os.path.join(directory, target["executable"]))
            if target["runtime"] == "linux/amd64":
                result = complete([binary, "--version"])
                if result.exit_code != 0 or result.stdout.strip() != f"fjx {version}":
                    fail(f"Linux runtime smoke failed for {target['triple']}")
        step([sys.executable, PACKAGE_SCRIPT, "publish-assets", "all", "--source-date-epoch", epoch],
             "package_release.py", env=child_env({"FJX_PACKAGE_BIN_ROOT": binaries}))
        verify_assets("publish-assets", version, source_date_epoch=int(epoch))
    finally:
        shutil.rmtree(binaries, ignore_errors=True)


def main_push() -> bool:
    return os.environ.get("FORGEJO_REF", "") == "refs/heads/main" and \
        os.environ.get("FORGEJO_EVENT_NAME", "") == "push"


def candidate(before: str, revision: str, output: str) -> None:
    os.chdir(PROJECT_DIR)
    os.environ.pop("RELEASE_WORKSPACE_ROOT", None)
    selected = release_candidate(before, revision) if main_push() else False
    with open(output, "a", encoding="utf-8", newline="") as file:
        file.write(f"release={'true' if selected else 'false'}\n")


def publish(revision: str) -> None:
    os.chdir(PROJECT_DIR)
    os.environ.pop("RELEASE_WORKSPACE_ROOT", None)
    if not main_push():
        fail("publication is only allowed for a push to main")
    if not release_candidate(os.environ.get("BEFORE_SHA", ""), revision):
        fail("publication requires a version increase across this push")
    epoch = git_output(["show", "-s", "--format=%ct", revision])
    # Reconcile remote state before building, and again immediately before writes.
    publication_state(revision, epoch)
    build_assets(revision)
    plan = publication_state(revision, epoch)
    # Create the draft and its assets before the tag exists. Forgejo publishes a
    # draft as soon as it syncs a pushed tag with the same name, so publish-release
    # pushes the tag only after the draft's asset names and sizes match the local
    # build. verify-final then compares every byte of the public release.
    release_id = str(create_release(plan["version"], plan["tag"])) if plan["needs_release"] else plan["release_id"]
    # A draft without a tag may hold assets from an earlier run. This run's
    # bytes can differ, so replace them all before the tag push.
    if plan["replace_assets"]:
        release_step("delete-draft-assets", release_id, revision)
    if plan["missing_assets"]:
        release_step("upload-missing-assets", release_id, plan["missing_assets"])
    if plan["needs_publication"]:
        local_tag = complete(["git", "rev-parse", "--quiet", "--verify", f"refs/tags/{plan['tag']}"])
        if plan["needs_tag"] and local_tag.exit_code != 0:
            step(["git", "-c", "user.name=Forgejo Actions", "-c", "user.email=actions@localhost", "tag",
                  "--annotate", plan["tag"], "--message", f"fjx v{plan['version']}", revision], "git")
        release_step("publish-release", plan["version"], revision, plan["tag"])
    release_step("verify-final", plan["version"], revision, plan["tag"])
    print(f"Published and verified {plan['tag']}.")


def run(argv: list[str]) -> None:
    if not argv:
        fail(USAGE)
    parser = ArgumentParser(prog="forgejo_ci.py", description="fjx Forgejo CI release steps")
    commands = parser.add_subparsers(dest="command", metavar="COMMAND", required=True)
    command = commands.add_parser("candidate", help="append release=true|false to OUTPUT")
    command.add_argument("before")
    command.add_argument("revision")
    command.add_argument("output", type=os.path.abspath)
    command = commands.add_parser("build", help="build and verify all release assets")
    command.add_argument("revision")
    command = commands.add_parser("publish", help="build, publish and verify the release")
    command.add_argument("revision")
    arguments = parser.parse_args(argv)
    if arguments.command == "candidate":
        candidate(arguments.before, arguments.revision, arguments.output)
    elif arguments.command == "build":
        build_assets(arguments.revision)
    else:
        publish(arguments.revision)


if __name__ == "__main__":
    sys.exit(cli_main(run))
