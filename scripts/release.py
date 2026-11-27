#!/usr/bin/env python3
"""Version files and Forgejo publication helpers for fjx releases.

Other release scripts import the shared helpers here: Failure, cli_main,
ArgumentParser, child_env, complete and checked.
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib
import urllib.parse
from typing import Any, NamedTuple

PROJECT_NAME = "fjx"
SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
# Child processes never need these. Git gets its credential through
# GIT_CONFIG_VALUE_0 and curl through its standard input.
TOKEN_VARIABLES = (
    "FJX_TOKEN",
    "FORGEJO_TOKEN",
    "GITEA_TOKEN",
    "GITHUB_TOKEN",
    "ACTIONS_RUNTIME_TOKEN",
    "GH_RELEASE_MIRROR_TOKEN",
)
VERSION_PATTERN = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)")
REPOSITORY_PATTERN = re.compile(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+")
TAG_PREFIX = "refs/tags/fjx/v"


# ---------------------------------------------------------------------------
# Shared script plumbing


class Failure(Exception):
    """A planned stop. The message is printed as `error: MESSAGE`."""

    def __init__(self, message: str, exit_code: int = 1):
        super().__init__(message)
        self.exit_code = exit_code


def fail(message: str):
    raise Failure(f"fjx release: {message}")


def secret_values() -> list[str]:
    values = []
    for name in TOKEN_VARIABLES:
        raw = os.environ.get(name, "")
        if raw.strip() == "":
            continue
        for value in (raw, raw.strip()):
            values.append(value)
            values.append(base64.b64encode(f"x-access-token:{value}".encode()).decode())
    return sorted(set(values), key=len, reverse=True)


def redact(text: str) -> str:
    for value in secret_values():
        text = text.replace(value, "[redacted]")
    return text


def report(message: str) -> None:
    sys.stdout.flush()
    sys.stderr.write(redact(f"error: {message}") + "\n")
    sys.stderr.flush()


def cli_main(run, argv: list[str] | None = None) -> int:
    """Runs a command and turns every failure into a plain, redacted error line."""
    try:
        run(sys.argv[1:] if argv is None else argv)
    except Failure as error:
        report(str(error))
        return error.exit_code
    except KeyboardInterrupt:
        return 130
    except Exception as error:  # Never print a traceback: it may hold values.
        report(f"unexpected {type(error).__name__}: {error}")
        return 1
    finally:
        sys.stdout.flush()
    return 0


class ArgumentParser(argparse.ArgumentParser):
    """argparse with exact flag names and exit code 1 on usage errors."""

    def __init__(self, *arguments, **options):
        options.setdefault("allow_abbrev", False)
        super().__init__(*arguments, **options)

    def error(self, message):
        self.print_usage(sys.stderr)
        raise Failure(f"{self.prog}: {message}")


def decimal_int(text: str) -> int:
    if not re.fullmatch(r"[+-]?[0-9]+", text):
        raise argparse.ArgumentTypeError(f"invalid integer: {text!r}")
    return int(text)


def child_env(extra: dict[str, str] | None = None) -> dict[str, str]:
    environment = {name: value for name, value in os.environ.items() if name not in TOKEN_VARIABLES}
    environment.update(extra or {})
    return environment


class Result(NamedTuple):
    exit_code: int
    stdout: str
    stderr: str


def decode(data: bytes | None) -> str:
    return (data or b"").decode("utf-8", errors="replace")


def complete(arguments: list[str], *, input: bytes | None = None, cwd: str | None = None,
             env: dict[str, str] | None = None) -> Result:
    """Runs a program, capturing its output and exit code without raising."""
    sys.stdout.flush()
    try:
        result = subprocess.run(arguments, input=input, capture_output=True, cwd=cwd,
                                env=child_env() if env is None else env)
    except OSError as error:
        raise Failure(f"could not run {arguments[0]}: {error.strerror}") from None
    return Result(result.returncode, decode(result.stdout), decode(result.stderr))


def checked(arguments: list[str], *, capture: bool = False, stdin=None, input: bytes | None = None,
            stdout=None, cwd: str | None = None, env: dict[str, str] | None = None) -> bytes:
    """Runs a program that must succeed. Output not captured goes to this process's streams."""
    sys.stdout.flush()
    try:
        result = subprocess.run(arguments, stdin=stdin, input=input,
                                stdout=subprocess.PIPE if capture else stdout, cwd=cwd,
                                env=child_env() if env is None else env)
    except OSError as error:
        raise Failure(f"could not run {arguments[0]}: {error.strerror}") from None
    if result.returncode != 0:
        name = os.path.basename(arguments[0])
        raise Failure(f"{name} failed with exit code {result.returncode}",
                      exit_code=result.returncode if result.returncode > 0 else 1)
    return result.stdout or b""


def lines(text: str) -> list[str]:
    """Splits like Nushell's `lines`: one trailing newline ends the last line."""
    if text == "":
        return []
    parts = text.split("\n")
    if parts[-1] == "":
        parts.pop()
    return [part[:-1] if part.endswith("\r") else part for part in parts]


def is_int(value: Any) -> bool:
    return type(value) is int


def field(record: Any, name: str) -> Any:
    """Reads an optional field like Nushell's `$record.name?`."""
    return record.get(name) if isinstance(record, dict) else None


def load_json(data: bytes) -> Any:
    """Parses strict JSON. Empty input is null, as with Nushell's `from json`."""
    if data == b"":
        return None

    def reject(constant):
        raise ValueError(f"invalid JSON constant {constant}")

    return json.loads(data.decode("utf-8"), parse_constant=reject)


def same(actual: Any, expected: Any) -> bool:
    """Compares values with their exact JSON types, unlike Python's 1 == True."""
    if type(actual) is not type(expected):
        return False
    if isinstance(expected, dict):
        return actual.keys() == expected.keys() and all(same(actual[key], expected[key]) for key in expected)
    if isinstance(expected, list):
        return len(actual) == len(expected) and all(same(a, e) for a, e in zip(actual, expected))
    return actual == expected


def sorted_names(names: list[Any]) -> list[str] | None:
    """Sorts names, or returns None when one is not a string."""
    if not all(isinstance(name, str) for name in names):
        return None
    return sorted(names)


def file_names(directory: str) -> list[str]:
    """Lists every non-directory entry, like Nushell's `glob DIR/* --no-dir`."""
    try:
        with os.scandir(directory) as entries:
            return sorted(entry.name for entry in entries if not entry.is_dir(follow_symlinks=False))
    except (FileNotFoundError, NotADirectoryError):
        return []


def remove(path: str | None) -> None:
    if path is None:
        return
    if os.path.isdir(path) and not os.path.islink(path):
        shutil.rmtree(path, ignore_errors=True)
    else:
        try:
            os.remove(path)
        except FileNotFoundError:
            pass


# ---------------------------------------------------------------------------
# Version files


def project_dir() -> str:
    # Explicit workspace overrides retain the historical parent/fjx fixture layout.
    workspace_root = os.environ.get("RELEASE_WORKSPACE_ROOT", "")
    if workspace_root:
        return os.path.join(workspace_root, PROJECT_NAME)
    return os.path.dirname(SCRIPT_DIR)


def valid_version(version: Any) -> bool:
    return isinstance(version, str) and VERSION_PATTERN.fullmatch(version) is not None


def version_greater(left: str, right: str) -> bool:
    a = [int(part) for part in left.split(".")]
    b = [int(part) for part in right.split(".")]
    return a > b


def checked_version(value: Any, label: str) -> str:
    if not valid_version(value):
        fail(f"invalid version in {label}")
    return value


def read_toml(path: str) -> dict:
    with open(path, "rb") as file:
        return tomllib.load(file)


TOML_ERRORS = (tomllib.TOMLDecodeError, UnicodeDecodeError, OSError, KeyError, TypeError)


def read_version_file(path: str, label: str) -> str:
    if not os.path.exists(path):
        fail(f"missing {label}")
    try:
        value = read_toml(path)["version"]
    except TOML_ERRORS:
        fail(f"invalid version in {label}")
    return checked_version(value, label)


def release_version() -> str:
    return read_version_file(os.path.join(project_dir(), "release.toml"), "release.toml")


def package_version() -> str:
    path = os.path.join(project_dir(), "Cargo.toml")
    if not os.path.exists(path):
        fail("missing Cargo.toml")
    try:
        value = read_toml(path)["package"]["version"]
    except TOML_ERRORS:
        fail("invalid version in Cargo.toml")
    return checked_version(value, "Cargo.toml")


def lock_version() -> str:
    path = os.path.join(project_dir(), "Cargo.lock")
    if not os.path.exists(path):
        fail("missing Cargo.lock")
    try:
        packages = read_toml(path)["package"]
    except TOML_ERRORS:
        fail("invalid fjx version in Cargo.lock")
    if not isinstance(packages, list):
        fail("invalid fjx version in Cargo.lock")
    matches = [package for package in packages if field(package, "name") == PROJECT_NAME]
    if len(matches) != 1:
        fail("invalid fjx version in Cargo.lock")
    value = matches[0].get("version")
    if not valid_version(value):
        fail("invalid fjx version in Cargo.lock")
    return value


def validate_versions() -> str:
    expected = release_version()
    cargo = package_version()
    lock = lock_version()
    if cargo != expected:
        fail(f"Cargo.toml version {cargo} does not match release.toml {expected}")
    if lock != expected:
        fail(f"Cargo.lock version {lock} does not match release.toml {expected}")
    return expected


def next_version(version: str, bump: str) -> str:
    major, minor, patch = (int(part) for part in version.split("."))
    if bump == "patch":
        return f"{major}.{minor}.{patch + 1}"
    if bump == "minor":
        return f"{major}.{minor + 1}.0"
    if bump == "major":
        return f"{major + 1}.0.0"
    fail("bump must be patch, minor, or major")


def rewrite_version(source: str, destination: str, version: str, lock: bool) -> None:
    with open(source, "rb") as file:
        try:
            text = file.read().decode("utf-8")
        except UnicodeDecodeError:
            fail(f"could not update {source}")
    if lock:
        pattern = r'(?ms)(\[\[package\]\]\s+name = "fjx"\s+version = ")[^"]+(".*)'
    else:
        pattern = r'(?m)^(version\s*=\s*")[^"]+(".*)$'
    output = re.sub(pattern, lambda match: match.group(1) + version + match.group(2), text, count=1)
    if output == text:
        fail(f"could not update {source}")
    with open(destination, "wb") as file:
        file.write(output.encode("utf-8"))


def rollback(files: list[str], transaction: str) -> None:
    for index, path in enumerate(files):
        original = os.path.join(transaction, f"original-{index}")
        # A missing copy means the failure came before any file was moved.
        if os.path.exists(original):
            shutil.copy2(original, path)
    shutil.rmtree(transaction, ignore_errors=True)


def prepare_version(bump: str, dry_run: bool = False) -> None:
    current = validate_versions()
    proposed = next_version(current, bump)
    result = "\n".join([
        f"project={PROJECT_NAME}",
        f"current_version={current}",
        f"proposed_version={proposed}",
        f"tag=fjx/v{proposed}",
        f"dry_run={'true' if dry_run else 'false'}",
    ])
    if dry_run:
        print(result)
        return

    project = project_dir()
    files = [os.path.join(project, name) for name in ("release.toml", "Cargo.toml", "Cargo.lock")]
    transaction = tempfile.mkdtemp(prefix=".release-prepare.", dir=project)
    try:
        for index, path in enumerate(files):
            shutil.copy2(path, os.path.join(transaction, f"original-{index}"))
        for index, path in enumerate(files):
            rewrite_version(path, os.path.join(transaction, f"staged-{index}"), proposed, index == 2)
        for index, path in enumerate(files):
            os.replace(os.path.join(transaction, f"staged-{index}"), path)
            if os.environ.get("RELEASE_WORKSPACE_ROOT", "") and \
                    os.environ.get("RELEASE_PREPARE_FAIL_AFTER_MOVE", "") == str(index + 1):
                fail("injected failure after move")
        if os.environ.get("RELEASE_PREPARE_FAIL_VALIDATION", "") == "true":
            fail("injected final validation failure")
        validate_versions()
    except BaseException:
        rollback(files, transaction)
        raise
    shutil.rmtree(transaction, ignore_errors=True)
    print(result)


def publication_plan(expected_revision: str, actual_revision: str, tag_status: int, source_date_epoch: str) -> dict:
    if actual_revision != expected_revision:
        fail(f"checked-out revision {actual_revision} does not match {expected_revision}")
    version = validate_versions()
    tag = f"fjx/v{version}"
    if tag_status == 0:
        return {"publish": False, "tag": tag, "version": version, "source_date_epoch": source_date_epoch}
    if tag_status == 2:
        return {"publish": True, "tag": tag, "version": version, "source_date_epoch": source_date_epoch}
    fail(f"could not determine whether {tag} exists; git ls-remote exited {tag_status}")


# ---------------------------------------------------------------------------
# Forgejo access. Tokens reach git only through GIT_CONFIG_VALUE_0 and curl
# only on standard input, never in argv, logs or errors.


def forgejo_env() -> tuple[str, str, str]:
    return (os.environ.get("FORGEJO_API_URL", "").strip(),
            os.environ.get("FORGEJO_REPOSITORY", "").strip(),
            os.environ.get("FORGEJO_TOKEN", "").strip())


def url_origin(value: str, label: str) -> tuple[str, str, int]:
    try:
        parsed = urllib.parse.urlsplit(value)
        explicit_port = parsed.port
        username, password = parsed.username, parsed.password
    except ValueError:
        fail(f"{label} is not a valid URL")
    scheme = parsed.scheme.lower()
    host = (parsed.hostname or "").lower()
    if scheme == "" or host == "":
        fail(f"{label} is not an absolute URL")
    if username or password:
        fail(f"{label} must not contain user information")
    if explicit_port is not None:
        port = explicit_port
    else:
        port = {"https": 443, "http": 80}.get(scheme, 0)
    return (scheme, host, port)


def forgejo_base(api: str) -> str:
    return re.sub(r"/api/v1/?\Z", "", api, count=1)


def validated_git_origin() -> str:
    api = os.environ.get("FORGEJO_API_URL", "").strip()
    repository = os.environ.get("FORGEJO_REPOSITORY", "").strip()
    if api == "" or not REPOSITORY_PATTERN.fullmatch(repository):
        fail("Forgejo API URL and repository are required before Git authentication")
    if url_origin(api, "Forgejo API URL")[0] != "https":
        fail("Git origin must use the exact HTTPS Forgejo origin before authentication")
    base = forgejo_base(api)
    if base == api:
        fail("Forgejo API URL must end with /api/v1")
    scope = f"{base}/"
    expected = f"{scope}{repository}.git"
    remote = complete(["git", "remote", "get-url", "origin"])
    if remote.exit_code != 0 or remote.stdout.strip() != expected:
        fail("Git origin must be the exact HTTPS Forgejo repository before authentication")
    return scope


def authenticated_git(arguments: list[str]) -> Result:
    token = os.environ.get("FORGEJO_TOKEN", "")
    if token.strip() == "":
        return complete(["git", *arguments])
    scope = validated_git_origin()
    credential = base64.b64encode(f"x-access-token:{token}".encode()).decode()
    return complete(["git", *arguments], env=child_env({
        "GIT_CONFIG_COUNT": "2",
        "GIT_CONFIG_KEY_0": f"http.{scope}.extraHeader",
        "GIT_CONFIG_VALUE_0": f"Authorization: Basic {credential}",
        "GIT_CONFIG_KEY_1": "http.followRedirects",
        "GIT_CONFIG_VALUE_1": "false",
    }))


def secret_header(token: str, scheme: str = "token") -> bytes:
    # A curl config line; curl reads it from standard input (`--config -`).
    return ("header = " + json.dumps(f"Authorization: {scheme} {token}", ensure_ascii=False) + "\n").encode("utf-8")


def curl_arguments(output: str, method: str, accept: str) -> list[str]:
    return ["curl", "--silent", "--show-error", "--proto", "=https", "--max-redirs", "0", "--config", "-",
            "--output", output, "--write-out", "%{http_code}", "--request", method, "--header", f"Accept: {accept}"]


def temporary_file() -> str:
    descriptor, path = tempfile.mkstemp()
    os.close(descriptor)
    return path


def release_api(method: str, path: str, *, attachment: str | None = None,
                body: dict | None = None) -> tuple[str, bytes]:
    """Sends one Forgejo API request. Returns the HTTP status text and response body."""
    api, repository, token = forgejo_env()
    if api == "" or repository == "" or token == "":
        fail("Forgejo API URL, repository, and token are required for publication state and repairs")
    if url_origin(api, "Forgejo API URL")[0] != "https":
        fail("Forgejo API URL must use HTTPS before authentication")
    response = temporary_file()
    body_file = None
    try:
        arguments = curl_arguments(response, method, "application/json")
        if attachment is not None:
            arguments += ["--form", f"attachment=@{attachment}"]
        if body is not None:
            body_file = temporary_file()
            with open(body_file, "wb") as file:
                file.write(json.dumps(body, separators=(",", ":"), ensure_ascii=False).encode("utf-8"))
            arguments += ["--header", "Content-Type: application/json", "--data-binary", f"@{body_file}"]
        arguments.append(f"{api}/repos/{repository}/{path}")
        request = complete(arguments, input=secret_header(token))
        remove(body_file)
        body_file = None
        if request.exit_code != 0:
            fail(f"Forgejo API request failed with exit code {request.exit_code}")
        with open(response, "rb") as file:
            content = file.read()
        return request.stdout.strip(), content
    finally:
        remove(body_file)
        remove(response)


def tag_path(tag: str) -> str:
    return f"releases/tags/{tag.replace('/', '%2F')}"


def checked_asset_url(asset: Any, name: str) -> str:
    """Checks an asset record's ID and its exact same-origin HTTPS download URL."""
    api, repository, token = forgejo_env()
    if api == "" or repository == "" or token == "":
        fail("Forgejo API URL, repository, and token are required to verify release assets")
    url = field(asset, "browser_download_url")
    url = url.strip() if isinstance(url, str) else ""
    if url == "":
        fail(f"release asset {name} has no download URL")
    api_origin = url_origin(api, "Forgejo API URL")
    asset_origin = url_origin(url, f"release asset {name} download URL")
    if asset_origin[0] != "https" or api_origin[0] != "https" or asset_origin != api_origin:
        fail(f"release asset {name} download URL must use the exact HTTPS Forgejo API origin")
    asset_id = field(asset, "id")
    if not is_int(asset_id) or asset_id <= 0:
        fail(f"release asset {name} has no valid Forgejo asset ID")

    # Forgejo's API attachment endpoint returns JSON metadata. Download the
    # release's canonical web URL, after checking the exact repository/tag/name.
    base = forgejo_base(api)
    tag = f"fjx%2Fv{validate_versions()}"
    request_url = f"{base}/{repository}/releases/download/{tag}/{name}"
    if url != request_url:
        fail(f"release asset {name} download URL must identify this exact repository, tag, and asset")
    return request_url


def release_assets(release: Any) -> list:
    assets = field(release, "assets")
    if assets is None:
        return []
    if not isinstance(assets, list):
        fail("Forgejo release assets are not a list; refusing ambiguous repair")
    return assets


def asset_name(asset: Any) -> Any:
    name = field(asset, "name")
    return "" if name is None else name


def verify_draft_assets(release: dict, version: str) -> None:
    # Forgejo serves draft asset bytes on its web routes only to users with write
    # access to releases. The Actions job token gets write access only on the API,
    # and the API returns asset metadata, not bytes. So a draft is checked by
    # metadata only: exact names, IDs, URLs, and sizes. verify-final compares the
    # bytes once the release is public.
    expected = asset_names(version)
    local = os.path.join(project_dir(), "publish-assets")
    if file_names(local) != expected:
        fail("locally checked release assets are unavailable or not exact")
    assets = release_assets(release)
    if sorted_names([asset_name(asset) for asset in assets]) != expected:
        fail("draft release asset set is not exact")
    for asset in assets:
        name = asset["name"]
        checked_asset_url(asset, name)
        size = field(asset, "size")
        local_size = os.path.getsize(os.path.join(local, name))
        if not is_int(size) or size != local_size:
            fail(f"draft release asset {name} size does not match the locally checked output")


def download_release_asset(asset: dict, name: str) -> str:
    token = os.environ.get("FORGEJO_TOKEN", "").strip()
    request_url = checked_asset_url(asset, name)
    downloaded = temporary_file()
    request = complete([*curl_arguments(downloaded, "GET", "application/octet-stream"), request_url],
                       input=secret_header(token))
    if request.exit_code != 0 or request.stdout.strip() != "200":
        remove(downloaded)
        fail(f"could not download release asset {name} without leaving the Forgejo origin")
    return downloaded


def inspect_remote_tags(tag: str, revision: str, version: str) -> bool:
    """Returns True when the tag still needs a push; fails on any unsafe tag state."""
    remote = authenticated_git(["ls-remote", "--tags", "origin", f"{TAG_PREFIX}*"])
    if remote.exit_code != 0:
        fail(f"could not list fjx release tags; git ls-remote exited {remote.exit_code}")
    refs = []
    for line in lines(remote.stdout):
        fields = [part for part in re.split(r"\s+", line) if part != ""]
        if len(fields) == 2:
            refs.append((fields[0], fields[1]))
    versions = []
    for _, ref in refs:
        ref = re.sub(r"\^\{\}\Z", "", ref, count=1)
        if ref.startswith(TAG_PREFIX) and ref not in versions:
            versions.append(ref)
    versions = [ref[len(TAG_PREFIX):] for ref in versions]
    for released in [candidate for candidate in versions if valid_version(candidate) and candidate != version]:
        if not version_greater(version, released):
            fail(f"release version {version} must be greater than prior released version {released}")
    tag_ref = f"refs/tags/{tag}"
    peeled_ref = f"{tag_ref}^{{}}"
    exact = [entry for entry in refs if entry[1] in (tag_ref, peeled_ref)]
    if not exact:
        return True
    peeled = [entry for entry in exact if entry[1] == peeled_ref]
    if len(peeled) != 1:
        fail(f"{tag} exists but is not one annotated tag; refusing to replace it")
    if peeled[0][0] != revision:
        fail(f"{tag} targets {peeled[0][0]} instead of {revision}; refusing to move it")
    return False


def release_targets_for_assets() -> list[dict]:
    path = os.path.join(project_dir(), "release-targets.toml")
    try:
        manifest = read_toml(path)
    except TOML_ERRORS:
        fail("invalid release-targets.toml")
    schema = manifest.get("schema")
    targets = manifest.get("targets")
    if not is_int(schema) or schema != 1 or not isinstance(targets, list) or len(targets) != 6:
        fail("release-targets.toml must declare schema 1 and exactly six targets")
    for target in targets:
        if not isinstance(target, dict) or not all(isinstance(target.get(key), str)
                                                   for key in ("triple", "archive", "executable")):
            fail("invalid release-targets.toml")
    return targets


def asset_names(version: str) -> list[str]:
    names = []
    for target in release_targets_for_assets():
        archive = f"fjx-{version}-{target['triple']}.{target['archive']}"
        names += [archive, f"{archive}.sha256"]
    return sorted(names)


def expected_release(version: str, tag: str, draft: bool = False) -> dict:
    return {"tag_name": tag, "name": f"fjx ({version})", "body": "", "draft": draft, "prerelease": False}


def archive_entries(archive_path: str, archive_format: str) -> list[str]:
    if archive_format == "tar.gz":
        output = checked(["tar", "-tzf", archive_path], capture=True)
    elif archive_format == "zip":
        output = checked(["unzip", "-Z1", archive_path], capture=True)
    else:
        fail(f"unsupported release archive format {archive_format}")
    return lines(decode(output))


def verify_downloaded_assets(directory: str, version: str) -> None:
    if file_names(directory) != asset_names(version):
        fail("downloaded release asset set is not exact")
    for target in release_targets_for_assets():
        archive = f"fjx-{version}-{target['triple']}.{target['archive']}"
        result = complete(["sha256sum", "--check", "--strict", f"{archive}.sha256"], cwd=directory)
        if result.exit_code != 0:
            fail(f"downloaded checksum failed for {archive}")
        if archive_entries(os.path.join(directory, archive), target["archive"]) != [target["executable"]]:
            fail(f"downloaded archive contents are invalid for {archive}")


def validate_release(release: Any, version: str, tag: str, draft: bool = False) -> None:
    actual = {key: field(release, key) for key in ("tag_name", "name", "body", "draft", "prerelease")}
    if not same(actual, expected_release(version, tag, draft)):
        fail("Forgejo release metadata is not exact; refusing ambiguous repair")


def verify_remote_assets(release: dict, version: str) -> None:
    expected = asset_names(version)
    local = os.path.join(project_dir(), "publish-assets")
    if file_names(local) != expected:
        fail("locally checked release assets are unavailable or not exact")
    downloaded = tempfile.mkdtemp()
    try:
        for asset in release_assets(release):
            name = asset["name"]
            destination = os.path.join(downloaded, name)
            shutil.move(download_release_asset(asset, name), destination)
            with open(os.path.join(local, name), "rb") as local_file, open(destination, "rb") as remote_file:
                if remote_file.read() != local_file.read():
                    fail(f"remote release asset {name} does not match the locally checked output")
        verify_downloaded_assets(downloaded, version)
    finally:
        shutil.rmtree(downloaded, ignore_errors=True)


def write_output(plan: dict, output: str) -> None:
    def text(value: Any) -> str:
        return ("true" if value else "false") if isinstance(value, bool) else str(value)

    publish = plan["publish"]
    entries = [
        ("publish", publish),
        ("needs_tag", plan.get("needs_tag", publish)),
        ("needs_release", plan.get("needs_release", publish)),
        ("needs_assets", plan.get("needs_assets", publish)),
        ("needs_publication", plan.get("needs_publication", publish)),
        ("replace_assets", plan.get("replace_assets", False)),
        ("missing_assets", plan.get("missing_assets", "")),
        ("release_id", plan.get("release_id", "")),
        ("tag", plan["tag"]),
        ("version", plan["version"]),
        ("source_date_epoch", plan["source_date_epoch"]),
    ]
    with open(output, "a", encoding="utf-8", newline="") as file:
        file.write("".join(f"{key}={text(value)}\n" for key, value in entries))


def head_revision() -> str:
    return decode(checked(["git", "rev-parse", "HEAD"], capture=True)).strip()


def read_release(content: bytes, message: str) -> Any:
    try:
        return load_json(content)
    except ValueError:
        fail(message)


def publication_state(expected_revision: str, source_date_epoch: str) -> dict:
    actual_revision = head_revision()
    if actual_revision != expected_revision:
        fail(f"checked-out revision {actual_revision} does not match {expected_revision}")
    version = validate_versions()
    tag = f"fjx/v{version}"
    needs_tag = inspect_remote_tags(tag, expected_revision, version)
    status, content = release_api("GET", tag_path(tag))
    release_id = ""
    needs_release = False
    needs_publication = True
    missing_assets: list[str] = []
    replace_assets = False
    if status == "200":
        release = read_release(content, "Forgejo returned an invalid release record")
        identity = field(release, "id")
        if field(release, "tag_name") != tag or not is_int(identity) or identity <= 0:
            fail("Forgejo returned an invalid release identity")
        draft = field(release, "draft")
        if not isinstance(draft, bool):
            fail("Forgejo release has no draft state")
        # The tag is pushed only after the draft is verified, so a draft without
        # a tag is an interrupted run. A public release without one is not.
        if needs_tag and not draft:
            fail(f"{tag} has a public Forgejo release but no verifiable remote tag; refusing ambiguous repair")
        validate_release(release, version, tag, draft)
        needs_publication = draft
        release_id = str(identity)
        assets = field(release, "assets")
        if assets is not None and not isinstance(assets, list):
            fail("release has duplicate or unexpected assets; refusing ambiguous repair")
        names = [asset_name(asset) for asset in assets or []]
        expected = asset_names(version)
        if not all(isinstance(name, str) for name in names) or len(set(names)) != len(names) or \
                any(name not in expected for name in names):
            fail("release has duplicate or unexpected assets; refusing ambiguous repair")
        # The job token cannot read draft bytes, so assets from an earlier run
        # cannot be checked. While the draft has no tag, replace them all. Once
        # the tag exists, Forgejo may publish the draft at any moment, so keep them.
        replace_assets = draft and needs_tag and len(names) > 0
        missing_assets = expected if replace_assets else [name for name in expected if name not in names]
        if not draft and missing_assets:
            fail("published release is incomplete; refusing to upload to a public release")
    elif status == "404":
        needs_release = True
        missing_assets = asset_names(version)
    else:
        fail(f"could not determine whether the Forgejo release exists (HTTP {status})")
    # A complete named asset set still needs a fresh local build so verify-final
    # can compare the remote bytes with output that passed the native checks.
    needs_assets = True
    complete_state = not (needs_tag or needs_release or needs_assets)
    return {
        "publish": not complete_state,
        "needs_tag": needs_tag,
        "needs_release": needs_release,
        "needs_publication": needs_publication,
        "needs_assets": needs_assets,
        "replace_assets": replace_assets,
        "missing_assets": ",".join(missing_assets),
        "release_id": release_id,
        "tag": tag,
        "version": version,
        "source_date_epoch": source_date_epoch,
    }


def create_release(version: str, tag: str) -> int:
    status, content = release_api("POST", "releases", body=expected_release(version, tag, draft=True))
    if status != "201":
        fail(f"Forgejo refused release creation (HTTP {status}); inspect remote state before rerunning")
    release = read_release(content, "Forgejo returned invalid JSON after creating the release; inspect remote state")
    validate_release(release, version, tag, draft=True)
    identity = field(release, "id")
    if not is_int(identity) or identity <= 0:
        fail("Forgejo returned an invalid release ID; inspect remote state before rerunning")
    return identity


def draft_without_tag(release_id: int, revision: str) -> dict:
    version = validate_versions()
    tag = f"fjx/v{version}"
    # Pushing the tag publishes the draft, so a pushed tag stops all deletes.
    if not inspect_remote_tags(tag, revision, version):
        fail(f"{tag} is already pushed; refusing to delete release assets")
    status, content = release_api("GET", tag_path(tag))
    if status != "200":
        fail(f"could not read the draft release (HTTP {status})")
    release = read_release(content, "Forgejo returned an invalid release record")
    identity = field(release, "id")
    if not is_int(identity) or identity != release_id:
        fail("draft release ID changed; inspect remote state before rerunning")
    validate_release(release, version, tag, draft=True)
    return release


def delete_draft_assets(release_id: int, revision: str) -> None:
    """Deletes every asset of a draft whose tag is not pushed.

    Each run rebuilds all assets and the job token cannot read draft bytes, so
    assets left by an earlier run are replaced, never trusted.
    """
    release = draft_without_tag(release_id, revision)
    expected = asset_names(validate_versions())
    assets = release_assets(release)
    for asset in assets:
        if asset_name(asset) not in expected:
            fail("draft has an unexpected asset; refusing ambiguous repair")
        if not is_int(asset.get("id")) or asset["id"] <= 0:
            fail(f"draft asset {asset['name']} has no valid Forgejo asset ID")
    for asset in assets:
        status, _ = release_api("DELETE", f"releases/{release_id}/assets/{asset['id']}")
        if status != "204":
            fail(f"Forgejo refused to delete draft asset {asset['name']} (HTTP {status}); "
                 "inspect remote state before rerunning")
    if release_assets(draft_without_tag(release_id, revision)):
        fail("draft still has assets after deletion; inspect remote state before rerunning")


def upload_missing_assets(release_id: int, names: str) -> None:
    expected = asset_names(validate_versions())
    for name in [name for name in names.split(",") if name != ""]:
        if name not in expected:
            fail(f"refusing unexpected attachment {name}")
        path = os.path.join(project_dir(), "publish-assets", name)
        if not os.path.exists(path):
            fail(f"missing prepared attachment {name}")
        status, _ = release_api("POST", f"releases/{release_id}/assets?name={name}", attachment=path)
        if status != "201":
            fail(f"Forgejo refused missing {name} attachment (HTTP {status}); no existing asset was overwritten")


def push_tag(tag: str, revision: str) -> None:
    tag_object = complete(["git", "cat-file", "-t", f"refs/tags/{tag}"])
    peeled = complete(["git", "rev-parse", f"refs/tags/{tag}^{{}}"])
    if tag_object.exit_code != 0 or tag_object.stdout.strip() != "tag" or \
            peeled.exit_code != 0 or peeled.stdout.strip() != revision:
        fail("refusing to push a missing, lightweight, or wrongly targeted release tag")
    if os.environ.get("FORGEJO_TOKEN", "").strip() == "":
        fail("Forgejo token is required to push a release tag")
    if authenticated_git(["push", "origin", f"refs/tags/{tag}"]).exit_code != 0:
        fail("authenticated release tag push failed")


def verified_release(version: str, revision: str, tag: str, draft: bool = False,
                     allow_missing_tag: bool = False) -> dict:
    if version != validate_versions() or tag != f"fjx/v{version}":
        fail("final release identity does not match project version")
    if inspect_remote_tags(tag, revision, version) and not allow_missing_tag:
        fail("final release tag is missing")
    status, content = release_api("GET", tag_path(tag))
    if status != "200":
        fail("final Forgejo release is missing")
    release = read_release(content, "final Forgejo release is invalid JSON")
    assets = field(release, "assets")
    names = sorted_names([asset_name(asset) for asset in assets]) if isinstance(assets, list) else \
        ([] if assets is None else None)
    validate_release(release, version, tag, draft)
    identity = field(release, "id")
    if not is_int(identity) or identity <= 0:
        fail("invalid final release ID")
    if names != asset_names(version):
        fail("final Forgejo release identity or assets are not exact")
    if draft:
        verify_draft_assets(release, version)
    else:
        verify_remote_assets(release, version)
    return release


def verify_final(version: str, revision: str, tag: str) -> None:
    verified_release(version, revision, tag)


def publish_release(version: str, revision: str, tag: str) -> None:
    """The only transition to a public release.

    The draft must first hold exactly the locally checked names and sizes. The
    job token cannot read draft bytes, so verify-final compares them after
    publication. An ambiguous PATCH stops; reruns read state. Forgejo also
    publishes a draft when its tag is pushed (a queued tag sync sets draft=false
    seconds later). So the tag is pushed only here, after the draft check and
    right before the PATCH. Either may publish first.
    """
    release = verified_release(version, revision, tag, draft=True, allow_missing_tag=True)
    if inspect_remote_tags(tag, revision, version):
        push_tag(tag, revision)
    status, content = release_api("PATCH", f"releases/{release['id']}", body={"draft": False})
    if status != "200":
        fail(f"Forgejo refused publication (HTTP {status}); inspect remote state before rerunning")
    published = read_release(content, "invalid publication response; inspect remote state before rerunning")
    validate_release(published, version, tag)
    identity = field(published, "id")
    if not is_int(identity) or identity != release["id"]:
        fail("published release ID changed; inspect remote state")


# ---------------------------------------------------------------------------
# Command line

USAGE = ("usage: scripts/release.py (version|validate|validate-tag TAG|prepare [--dry-run] BUMP|"
         "publish-plan REVISION EPOCH OUTPUT|push-tag TAG REVISION|delete-draft-assets RELEASE_ID REVISION|"
         "upload-missing-assets RELEASE_ID NAMES|publish-release VERSION REVISION TAG|"
         "verify-final VERSION REVISION TAG)")


def validate_tag(tag: str) -> None:
    expected = f"fjx/v{validate_versions()}"
    if tag != expected:
        fail(f"tag must be {expected}")


def run(argv: list[str]) -> None:
    if not argv:
        fail(USAGE)
    parser = ArgumentParser(prog="release.py", description="fjx version and Forgejo release helpers")
    commands = parser.add_subparsers(dest="command", metavar="COMMAND")
    commands.add_parser("version", help="print release.toml's version")
    commands.add_parser("validate", help="check that the version files agree")
    command = commands.add_parser("validate-tag", help="check a release tag name")
    command.add_argument("tag")
    command = commands.add_parser("prepare", help="bump the version in all version files")
    command.add_argument("--dry-run", action="store_true")
    command.add_argument("bump")
    command = commands.add_parser("publish-plan", help="append the publication plan to OUTPUT")
    command.add_argument("expected_revision")
    command.add_argument("source_date_epoch")
    command.add_argument("output", type=os.path.abspath)
    command = commands.add_parser("push-tag", help="push one annotated release tag")
    command.add_argument("tag")
    command.add_argument("revision")
    command = commands.add_parser("delete-draft-assets", help="delete all assets of an untagged draft")
    command.add_argument("release_id", type=decimal_int)
    command.add_argument("revision")
    command = commands.add_parser("upload-missing-assets", help="upload the named prepared assets")
    command.add_argument("release_id", type=decimal_int)
    command.add_argument("names")
    for name in ("publish-release", "verify-final"):
        command = commands.add_parser(name)
        command.add_argument("version")
        command.add_argument("revision")
        command.add_argument("tag")
    arguments = parser.parse_args(argv)

    if arguments.command == "version":
        print(release_version())
    elif arguments.command == "validate":
        print(validate_versions())
    elif arguments.command == "validate-tag":
        validate_tag(arguments.tag)
    elif arguments.command == "prepare":
        prepare_version(arguments.bump, arguments.dry_run)
    elif arguments.command == "publish-plan":
        write_output(publication_state(arguments.expected_revision, arguments.source_date_epoch), arguments.output)
    elif arguments.command == "push-tag":
        push_tag(arguments.tag, arguments.revision)
    elif arguments.command == "delete-draft-assets":
        delete_draft_assets(arguments.release_id, arguments.revision)
    elif arguments.command == "upload-missing-assets":
        upload_missing_assets(arguments.release_id, arguments.names)
    elif arguments.command == "publish-release":
        publish_release(arguments.version, arguments.revision, arguments.tag)
    elif arguments.command == "verify-final":
        verify_final(arguments.version, arguments.revision, arguments.tag)


if __name__ == "__main__":
    sys.exit(cli_main(run))
