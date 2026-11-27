#!/usr/bin/env python3
"""Copies public Forgejo releases to GitHub and verifies every byte.

Repository automation, not a second release builder. Forgejo owns the bytes.
"""

from __future__ import annotations

import hashlib
import json
import os
import re
import shutil
import sys
import tempfile
import time
import urllib.parse
from typing import Any

from release import (ArgumentParser, Failure, child_env, cli_main, complete, field, is_int, lines, load_json,
                     same, secret_header)
from package_release import archive_name, release_targets

SOURCE = "https://dyad.tail77425.ts.net/forgejo"
SOURCE_REPO = "dalton/fjx"
DESTINATION = "DaltonAlley/fjx"
GITHUB_HOSTS = ("api.github.com", "uploads.github.com")
CDN_HOSTS = ("release-assets.githubusercontent.com", "objects.githubusercontent.com")
TAG_PATTERN = re.compile(r"fjx/v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z.-]+)?")
LOCATION_PATTERN = re.compile(r"location:\s*([^\r]+)", re.IGNORECASE)
PAGE_SIZE = 100
MAX_RECORDS = 10_000
METADATA_FIELDS = ("tag_name", "name", "body", "prerelease")
INVENTORY_FIELDS = ("id", "name", "size", "browser_download_url")


def fail(message: str):
    raise Failure(f"fjx GitHub mirror: {message}")


def temporary(directory: str, prefix: str) -> str:
    descriptor, path = tempfile.mkstemp(prefix=prefix, dir=directory)
    os.close(descriptor)
    return path


def request(method: str, url: str, directory: str, *, body: dict | None = None, file: str | None = None,
            binary: bool = False, anonymous: bool = False) -> dict:
    """Sends one HTTPS request with curl.

    Tokens travel on stdin, never argv. Authenticated requests never redirect.
    Do not print curl errors or response bodies: either may contain credentials.
    """
    # Only plain printable URLs, so curl and this parser read the same host.
    if re.search(r"[\s\\\x00-\x1f\x7f]", url):
        fail("request must use HTTPS without user information")
    try:
        parsed = urllib.parse.urlsplit(url)
        host = (parsed.hostname or "").lower()
        user, password = parsed.username, parsed.password
    except ValueError:
        fail("request must use HTTPS without user information")
    if parsed.scheme.lower() != "https" or user or password:
        fail("request must use HTTPS without user information")
    github = host in GITHUB_HOSTS
    forgejo = url.startswith(f"{SOURCE}/")
    if not anonymous and not (github or forgejo):
        fail("refusing authentication outside the configured services")
    if anonymous:
        token = ""
    elif github:
        token = os.environ.get("GH_RELEASE_MIRROR_TOKEN", "")
    else:
        token = os.environ.get("FORGEJO_TOKEN", "")
    if not anonymous and token.strip() == "":
        fail("required mirror credential is missing")
    if "\r" in token or "\n" in token:
        fail("invalid mirror credential")
    output = temporary(directory, "response.")
    headers = temporary(directory, "headers.")
    accept = "application/octet-stream" if binary else "application/json"
    arguments = ["curl", "--disable", "--silent", "--show-error", "--proto", "=https", "--max-redirs", "0",
                 "--connect-timeout", "20", "--max-time", "300", "--config", "-", "--output", output,
                 "--dump-header", headers, "--write-out", "%{http_code}", "--request", method,
                 "--header", f"Accept: {accept}"]
    if body is not None:
        data = temporary(directory, "body.")
        with open(data, "wb") as handle:
            handle.write(json.dumps(body, separators=(",", ":"), ensure_ascii=False).encode("utf-8"))
        arguments += ["--header", "Content-Type: application/json", "--data-binary", f"@{data}"]
    if file is not None:
        arguments += ["--header", "Content-Type: application/octet-stream", "--data-binary", f"@{file}"]
    configuration = b"" if anonymous else secret_header(token, "Bearer" if github else "token")
    # child_env() drops every token: curl reads the one it needs from stdin.
    result = complete([*arguments, url], input=configuration, env=child_env())
    if result.exit_code != 0:
        fail("HTTP transfer failed; rerun to reconcile remote state before retrying writes")
    return {"status": result.stdout.strip(), "output": output, "headers": headers}


def api(service: str, method: str, path: str, directory: str, body: dict | None = None) -> Any:
    if service == "forgejo":
        base = f"{SOURCE}/api/v1/repos/{SOURCE_REPO}"
    else:
        base = f"https://api.github.com/repos/{DESTINATION}"
    result = request(method, f"{base}/{path}", directory, body=body)
    if result["status"] not in ("200", "201", "204"):
        fail(f"{service} {method} failed with HTTP {result['status']}; "
             "inspect the run and rerun after resolving the failure")
    if result["status"] == "204":
        return None
    try:
        with open(result["output"], "rb") as handle:
            return load_json(handle.read())
    except (ValueError, RecursionError):
        fail(f"{service} returned invalid JSON")


def list_all(service: str, path: str, directory: str) -> list[dict]:
    """Reads every page of a listing.

    Forgejo caps pages at its max_response_items (50 by default), below the
    requested limit, so a short Forgejo page does not mean the last page; read
    until an empty one. GitHub honours per_page, so a short page ends it.
    """
    items: list[dict] = []
    page = 1
    while len(items) <= MAX_RECORDS:
        batch = api(service, "GET", f"{path}?per_page={PAGE_SIZE}&page={page}&limit={PAGE_SIZE}", directory)
        if not isinstance(batch, list):
            fail("expected a paginated array")
        # Every caller reads fields, so every entry must be an object.
        if not all(isinstance(item, dict) for item in batch):
            fail("expected a paginated array of objects")
        items += batch
        if not batch or (service == "github" and len(batch) < PAGE_SIZE):
            return items
        page += 1
    fail("pagination exceeded 10,000 records; refusing an incomplete mirror")


def text_field(record: dict, name: str) -> str:
    value = field(record, name)
    if value is None:
        return ""
    if not isinstance(value, str):
        fail(f"release {name} must be text")
    return value


def release_identity(release: Any) -> dict:
    if not isinstance(field(release, "draft"), bool) or not isinstance(field(release, "prerelease"), bool):
        fail("release must declare draft and prerelease state")
    tag = field(release, "tag_name")
    if not isinstance(tag, str) or not TAG_PATTERN.fullmatch(tag):
        fail("release tag must use fjx/vVERSION")
    version = tag.removeprefix("fjx/v")
    if "-" in version and not release["prerelease"]:
        fail("prerelease version must be marked as a prerelease")
    identity = field(release, "id")
    if not is_int(identity) or identity <= 0:
        fail("release has no valid ID")
    return {"tag_name": tag, "name": text_field(release, "name"), "body": text_field(release, "body"),
            "prerelease": release["prerelease"]}


def asset_names(tag: str) -> list[str]:
    version = tag.removeprefix("fjx/v")
    names = []
    for target in release_targets():
        name = archive_name(version, target)
        names += [name, f"{name}.sha256"]
    return sorted(names)


def assets_of(release: Any) -> list:
    assets = field(release, "assets")
    if not isinstance(assets, list):
        fail("release assets must be a list")
    return assets


def validate_assets(assets: list, tag: str, partial: bool = False) -> None:
    names = [field(asset, "name") for asset in assets]
    expected = asset_names(tag)
    if not all(isinstance(name, str) for name in names) or len(set(names)) != len(names) or \
            any(name not in expected for name in names):
        fail("duplicate or unexpected release assets; refusing to overwrite or delete them")
    if not partial and sorted(names) != expected:
        fail("published source release does not have all twelve assets")
    for asset in assets:
        identity, size = asset.get("id"), asset.get("size")
        if not is_int(identity) or identity <= 0 or not is_int(size) or size <= 0:
            fail("asset has an invalid ID or size")


def tag_refs(url: str, tag: str) -> list[str]:
    result = complete(["git", "ls-remote", "--tags", url, f"refs/tags/{tag}", f"refs/tags/{tag}^{{}}"])
    if result.exit_code != 0:
        fail("could not read remote release tag")
    return sorted(lines(result.stdout))


def tag_wait_seconds() -> int:
    value = os.environ.get("FJX_MIRROR_TAG_WAIT_SECONDS", "300")
    if not re.fullmatch(r"[0-9]+", value):
        fail("FJX_MIRROR_TAG_WAIT_SECONDS must be a whole number of seconds")
    return int(value)


def verify_tag(tag: str, wait: bool = False) -> None:
    github = f"https://github.com/{DESTINATION}.git"
    expected = tag_refs(f"{SOURCE}/{SOURCE_REPO}.git", tag)
    actual = tag_refs(github, tag)
    # Right after a release, the native push mirror may not have sent the new
    # tag yet. Wait a bounded time for a missing tag (read-only). A different
    # tag hash still fails at once.
    if wait and expected and not actual:
        limit = tag_wait_seconds()
        interval = min(limit, 10)
        deadline = time.monotonic() + limit
        while not actual and time.monotonic() < deadline:
            time.sleep(interval)
            actual = tag_refs(github, tag)
    if not expected or actual != expected:
        fail("release tag hashes differ; rerun after the native code mirror succeeds "
             "(tags are never created or moved here)")


def download_source(asset: dict, tag: str, destination: str, directory: str) -> None:
    encoded = tag.replace("/", "%2F")
    expected = f"{SOURCE}/{SOURCE_REPO}/releases/download/{encoded}/{asset['name']}"
    if field(asset, "browser_download_url") != expected:
        fail("source asset URL must identify this exact repository, tag, and filename")
    response = request("GET", expected, directory, binary=True)
    if response["status"] != "200":
        fail("could not download source asset without redirecting authentication")
    shutil.move(response["output"], destination)


def download_github(asset: dict, target: str, directory: str) -> None:
    response = request("GET", f"https://api.github.com/repos/{DESTINATION}/releases/assets/{asset['id']}",
                       directory, binary=True)
    if response["status"] == "200":
        shutil.move(response["output"], target)
        return
    if response["status"] != "302":
        fail("could not download GitHub asset for verification")
    with open(response["headers"], "rb") as handle:
        header_lines = lines(handle.read().decode("utf-8", errors="replace"))
    locations = [match.group(1) for match in map(LOCATION_PATTERN.fullmatch, header_lines) if match]
    if len(locations) != 1:
        fail("GitHub asset redirect has no unique location")
    url = locations[0].strip()
    try:
        parsed = urllib.parse.urlsplit(url)
        host = (parsed.hostname or "").lower()
    except ValueError:
        fail("GitHub asset redirect must use the GitHub release CDN")
    if parsed.scheme.lower() != "https" or host not in CDN_HOSTS:
        fail("GitHub asset redirect must use the GitHub release CDN")
    # The signed CDN request receives no GitHub or Forgejo authorization header.
    downloaded = request("GET", url, directory, binary=True, anonymous=True)
    if downloaded["status"] != "200":
        fail("GitHub release CDN download failed")
    shutil.move(downloaded["output"], target)


def digest(path: str) -> str:
    with open(path, "rb") as handle:
        return hashlib.sha256(handle.read()).hexdigest()


def verify_checksums(directory: str, tag: str) -> None:
    for name in asset_names(tag):
        if name.endswith(".sha256"):
            continue
        with open(os.path.join(directory, f"{name}.sha256"), "rb") as handle:
            checksum = handle.read().decode("utf-8", errors="replace").strip()
        if checksum != f"{digest(os.path.join(directory, name))}  {name}":
            fail("source archive checksum does not match its checksum file")


def same_metadata(release: Any, identity: dict) -> bool:
    """Compares notes, name, tag and prerelease state. A missing body is empty."""
    if not isinstance(release, dict) or any(name not in release for name in METADATA_FIELDS):
        return False
    actual = {name: release[name] for name in METADATA_FIELDS}
    if actual["body"] is None:
        actual["body"] = ""
    return same(actual, identity)


def inventory(assets: Any) -> list[dict] | None:
    """The asset fields that identify content: no download counters."""
    if not isinstance(assets, list) or not all(isinstance(asset, dict) and
                                               all(name in asset for name in INVENTORY_FIELDS)
                                               for asset in assets):
        return None
    if not all(isinstance(asset["name"], str) for asset in assets):
        return None
    return [{name: asset[name] for name in INVENTORY_FIELDS} for asset in sorted(assets, key=lambda a: a["name"])]


def sync_one(source_release: dict, directory: str) -> None:
    identity = release_identity(source_release)
    tag = identity["tag_name"]
    source_assets = assets_of(source_release)
    validate_assets(source_assets, tag)
    verify_tag(tag, wait=True)
    assets_directory = os.path.join(directory, "source-assets")
    downloads = os.path.join(directory, "github-assets")
    os.makedirs(assets_directory)
    os.makedirs(downloads)
    for asset in source_assets:
        path = os.path.join(assets_directory, asset["name"])
        download_source(asset, tag, path, directory)
        if os.path.getsize(path) != asset["size"]:
            fail("source asset size changed during download")
    verify_checksums(assets_directory, tag)

    # Listing includes drafts; the tags endpoint alone can miss a partial draft.
    candidates = [entry for entry in list_all("github", "releases", directory) if entry.get("tag_name") == tag]
    if len(candidates) > 1:
        fail("multiple GitHub releases use the same tag")
    if candidates:
        release = candidates[0]
    else:
        release = api("github", "POST", "releases", directory, body={**identity, "draft": True, "make_latest": "false"})
    release_id = field(release, "id")
    if not is_int(release_id) or release_id <= 0 or field(release, "tag_name") != tag or \
            not isinstance(field(release, "draft"), bool):
        fail("invalid GitHub release identity")
    listed = list_all("github", f"releases/{release_id}/assets", directory)
    validate_assets(listed, tag, partial=True)
    expected_count = len(asset_names(tag))
    if not release["draft"] and len(listed) != expected_count:
        fail("existing public GitHub release is incomplete; explicit repair is required")
    # A draft may hold assets from an earlier, interrupted copy. Replace them
    # all, so stale bytes are never published. A public release is never changed.
    if release["draft"] and listed:
        for asset in listed:
            api("github", "DELETE", f"releases/assets/{asset['id']}", directory)
        if list_all("github", f"releases/{release_id}/assets", directory):
            fail("draft assets remain after deletion; rerun to reconcile the release")
        assets = []
    else:
        assets = listed
    present = {asset["name"] for asset in assets}
    for source_asset in source_assets:
        name = source_asset["name"]
        if name not in present:
            # Only missing names are uploaded. An uncertain response stops this run;
            # the next run replaces the draft's assets before publishing.
            uploaded = request("POST", f"https://uploads.github.com/repos/{DESTINATION}/releases/{release_id}"
                               f"/assets?name={name}", directory, file=os.path.join(assets_directory, name))
            if uploaded["status"] != "201":
                fail("asset upload failed; rerun to reconcile the existing release")
    final_assets = list_all("github", f"releases/{release_id}/assets", directory)
    validate_assets(final_assets, tag)
    for asset in final_assets:
        downloaded = os.path.join(downloads, asset["name"])
        download_github(asset, downloaded, directory)
        if digest(downloaded) != digest(os.path.join(assets_directory, asset["name"])):
            fail("GitHub asset differs from Forgejo; refusing to replace it or publish the draft")
    # A source edit or withdrawal while uploading must not be published blindly.
    current = api("forgejo", "GET", f"releases/{source_release['id']}", directory)
    if field(current, "draft") is not False or not same_metadata(current, identity) or \
            inventory(field(current, "assets")) is None or \
            not same(inventory(field(current, "assets")), inventory(source_assets)):
        fail("source release changed while copying; rerun to reconcile it")
    verify_tag(tag)
    if release["draft"] or not same_metadata(release, identity):
        latest = "false" if identity["prerelease"] else "legacy"
        api("github", "PATCH", f"releases/{release_id}", directory,
            body={**identity, "draft": False, "make_latest": latest})
    final = api("github", "GET", f"releases/{release_id}", directory)
    if field(final, "draft") is not False or not same_metadata(final, identity):
        fail("final release metadata differs from Forgejo")
    print(f"Verified {tag}: notes, prerelease state, tag hashes, and twelve identical assets.", flush=True)


def mirror() -> None:
    directory = tempfile.mkdtemp()
    try:
        releases = list_all("forgejo", "releases", directory)
        for release in releases:
            if not isinstance(release.get("draft"), bool):
                fail("release must declare draft and prerelease state")
        public = [release for release in releases if release["draft"] is False]
        for release in public:
            if not is_int(release.get("id")) or release["id"] <= 0:
                fail("release has no valid ID")
        for release in sorted(public, key=lambda entry: entry["id"]):
            sync_one(release, tempfile.mkdtemp(prefix="release.", dir=directory))
    finally:
        shutil.rmtree(directory, ignore_errors=True)


def run(argv: list[str]) -> None:
    ArgumentParser(prog="github_mirror.py", description="Copy public Forgejo releases to GitHub").parse_args(argv)
    mirror()


if __name__ == "__main__":
    sys.exit(cli_main(run))
