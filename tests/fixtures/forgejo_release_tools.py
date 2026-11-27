"""Simulated external tools for the candidate/build/publish entry points.

Port of forgejo-release-tools.nu. Git ancestry, packaging, checksums,
comparisons and local smoke calls stay real. Each tool keeps its state in
FAKE_STATE: `calls` (JSON lines of {event, value}), `tag`, `release.json`,
`assets/`, `pending-tag-sync` and `next-asset-id`.

FAKE_STRICT_CURL=true also makes curl refuse a call that lacks the HTTPS-only
and no-redirect flags, carries a credential in argv or the environment, or (with
FAKE_EXPECTED_TOKEN) lacks the exact token header on standard input.
"""

import json
import os
import re
import subprocess
import sys
import time
import tomllib

TOKEN_VARIABLES = ("FJX_TOKEN", "FORGEJO_TOKEN", "GITEA_TOKEN", "GITHUB_TOKEN", "ACTIONS_RUNTIME_TOKEN",
                   "GH_RELEASE_MIRROR_TOKEN")


class Stop(Exception):
    pass


def state_path(*parts):
    return os.path.join(os.environ["FAKE_STATE"], *parts)


def log(event, value=""):
    with open(state_path("calls"), "a", encoding="utf-8") as file:
        file.write(json.dumps({"event": event, "value": value}) + "\n")


def argument(args, flag):
    return args[args.index(flag) + 1]


def read_json(path):
    with open(path, encoding="utf-8") as file:
        return json.load(file)


def write_json(path, value):
    with open(path, "w", encoding="utf-8") as file:
        json.dump(value, file, indent=2)


def run(function, args):
    try:
        return function(args)
    except Stop as error:
        sys.stderr.write(f"{error}\n")
        return 1


def git(args):
    return run(_git, args)


def _git(args):
    tag = state_path("tag")
    real_git = os.environ["FAKE_REAL_GIT"]
    if args[:3] == ["remote", "get-url", "origin"]:
        print("https://forge.example/forgejo/dalton/fjx.git")
        return 0
    if args[0] == "ls-remote":
        if os.path.exists(tag):
            with open(tag, encoding="utf-8") as file:
                sys.stdout.write(file.read())
        return 0
    if args[0] == "push":
        reference = args[-1]
        obj = subprocess.run([real_git, "rev-parse", reference], capture_output=True, text=True, check=True)
        revision = subprocess.run([real_git, "rev-parse", f"{reference}^{{}}"], capture_output=True, text=True,
                                  check=True)
        with open(tag, "w", encoding="utf-8") as file:
            file.write(f"{obj.stdout.strip()} {reference}\n{revision.stdout.strip()} {reference}^{{}}\n")
        log("push-tag")
        # Forgejo syncs pushed tags in a queue. The sync publishes any draft with
        # that tag name. In v0.4.0 and v0.5.0 it ran 2-3 s after the push, once the
        # draft existed. Model that: it runs on the first API call after a release
        # exists. FAKE_TAG_SYNC=never models a queue slower than the job.
        if os.environ.get("FAKE_TAG_SYNC", "queued") != "never":
            open(state_path("pending-tag-sync"), "a").close()
        return 0
    return subprocess.run([real_git, *args]).returncode


def cargo(args):
    return run(_cargo, args)


def _cargo(args):
    for name in TOKEN_VARIABLES:
        if name in os.environ:
            raise Stop("job credential inherited by build")
    if args[:4] != ["zigbuild", "--locked", "--release", "--target"]:
        raise Stop("unexpected build command")
    triple = args[-1]
    log("build", triple)
    if os.environ.get("FAKE_FAIL_BUILD", "") == triple:
        return 1
    project = os.environ["FAKE_PROJECT"]
    with open(os.path.join(project, "release-targets.toml"), "rb") as file:
        target = next(entry for entry in tomllib.load(file)["targets"] if entry["triple"] == triple)
    with open(os.path.join(project, "release.toml"), "rb") as file:
        version = tomllib.load(file)["version"]
    directory = os.path.join(os.environ["CARGO_TARGET_DIR"], triple, "release")
    os.makedirs(directory, exist_ok=True)
    binary = os.path.join(directory, target["executable"])
    description = {
        "x86_64-unknown-linux-gnu": "ELF 64-bit LSB x86-64 dynamically linked",
        "aarch64-unknown-linux-gnu": "ELF 64-bit LSB ARM aarch64 dynamically linked",
        "x86_64-unknown-linux-musl": "ELF 64-bit LSB x86-64 statically linked",
        "aarch64-unknown-linux-musl": "ELF 64-bit LSB ARM aarch64 statically linked",
        "aarch64-apple-darwin": "Mach-O 64-bit arm64 executable",
        "x86_64-pc-windows-gnu": "PE32+ executable MS Windows x86-64",
    }[triple]
    script = f"#!/usr/bin/env bash\n# format: {description}\nprintf 'fjx {version}\\n'\n"
    with open(binary, "wb") as file:
        if target["os"] == "windows":
            # Like the real linker: the PE timestamp is SOURCE_DATE_EPOCH, else now.
            # FAKE_UNPINNED_LINK models a linker that ignores SOURCE_DATE_EPOCH.
            pinned = None if os.environ.get("FAKE_UNPINNED_LINK", "") == "true" else os.environ.get("SOURCE_DATE_EPOCH")
            stamp = int(pinned) if pinned is not None else int(time.time())
            file.write(pe_header(stamp) + b"\n" + script.encode())
        else:
            file.write(script.encode())
    os.chmod(binary, 0o755)
    return 0


def pe_header(stamp):
    """A minimal MZ/PE header whose COFF TimeDateStamp is stamp."""
    return b"MZ" + bytes(58) + bytes([0x40, 0, 0, 0, 0x50, 0x45, 0, 0, 0x64, 0x86, 0x06, 0]) + \
        stamp.to_bytes(8, "little", signed=True)[:4]


def file(args):
    return run(_file, args)


def _file(args):
    log("format-check")
    with open(args[-1], "rb") as binary:
        for line in binary.read().split(b"\n"):
            if line.startswith(b"# format: "):
                print(line[len(b"# format: "):].decode())
                return 0
    raise Stop("no format line")


def make_public(record):
    """Logs every draft-to-public change with the asset count at that moment."""
    release = read_json(record)
    if release["draft"]:
        log("public", str(len(release["assets"])))
    release["draft"] = False
    write_json(record, release)


def run_tag_sync(record):
    pending = state_path("pending-tag-sync")
    if not os.path.exists(pending) or not os.path.exists(record):
        return
    os.remove(pending)
    log("tag-sync")
    make_public(record)


def check_curl_call(args):
    if os.environ.get("FAKE_STRICT_CURL", "") != "true":
        return
    joined = " ".join(args)
    for flags in (["--proto", "=https"], ["--max-redirs", "0"], ["--config", "-"]):
        if not any(args[index:index + 2] == flags for index in range(len(args))):
            raise Stop(f"unsafe curl call: missing {' '.join(flags)}")
    if "--location" in args or "-L" in args:
        raise Stop("unsafe curl call: follows redirects")
    leaked = [name for name in TOKEN_VARIABLES if name in os.environ]
    if leaked:
        raise Stop(f"unsafe curl call: credential in environment {leaked}")
    token = os.environ.get("FAKE_EXPECTED_TOKEN", "")
    header = sys.stdin.buffer.read()
    if token:
        if token in joined:
            raise Stop("unsafe curl call: token in arguments")
        if header != ("header = " + json.dumps(f"Authorization: token {token}") + "\n").encode():
            raise Stop("unsafe curl call: token header is not on standard input")


def curl(args):
    return run(_curl, args)


def _curl(args):
    check_curl_call(args)
    output = argument(args, "--output")
    method = argument(args, "--request")
    url = args[-1]
    record = state_path("release.json")
    assets = state_path("assets")
    run_tag_sync(record)
    log(method, url)
    # Like Forgejo 17 with the Actions job token: the web download routes give
    # it public read access only, so draft asset bytes are 404 there. The API
    # gives it write access but returns asset metadata, never bytes.
    if re.search("/releases/download/", url) or re.search("/attachments/", url):
        name = os.path.basename(url)
        log("download", name)
        if read_json(record)["draft"]:
            sys.stdout.write("404")
            return 0
        with open(os.path.join(assets, name), "rb") as source:
            data = source.read()
        if os.environ.get("FAKE_CORRUPT_DOWNLOAD", "") == "true":
            # Same size, different bytes: only a byte comparison can catch it.
            data = data[::-1]
        with open(output, "wb") as destination:
            destination.write(data)
        sys.stdout.write("200")
        return 0
    if method == "GET":
        if not os.path.exists(record):
            sys.stdout.write("404")
            return 0
        release = read_json(record)
        # FAKE_DRAFT_METADATA models a draft whose first asset record is wrong.
        if release["draft"] and release["assets"]:
            first = release["assets"][0]
            mode = os.environ.get("FAKE_DRAFT_METADATA", "")
            if mode == "size":
                first["size"] -= 1
            elif mode == "origin":
                first["browser_download_url"] = first["browser_download_url"].replace(
                    "https://forge.example", "https://evil.example")
            elif mode == "id":
                del first["id"]
        write_json(output, release)
        sys.stdout.write("200")
        return 0
    if method == "POST" and url.endswith("/releases"):
        with open(argument(args, "--data-binary")[1:], encoding="utf-8") as body_file:
            body = json.load(body_file)
        write_json(record, {**body, "id": 42, "assets": []})
        write_json(output, read_json(record))
        sys.stdout.write("201")
        return 0
    if method == "POST":
        name = url.split("?name=")[-1]
        if os.environ.get("FAKE_FAIL_UPLOAD", "") == name:
            sys.stdout.write("503")
            return 0
        release = read_json(record)
        if name in [asset["name"] for asset in release["assets"]]:
            raise Stop("existing asset overwritten")
        source = argument(args, "--form").replace("attachment=@", "", 1)
        with open(source, "rb") as attachment, open(os.path.join(assets, name), "wb") as stored:
            stored.write(attachment.read())
        size = os.path.getsize(os.path.join(assets, name))
        # Forgejo never reuses an attachment ID, even after a delete.
        counter = state_path("next-asset-id")
        if os.path.exists(counter):
            with open(counter, encoding="utf-8") as counter_file:
                asset_id = int(counter_file.read())
        else:
            asset_id = 1
        with open(counter, "w", encoding="utf-8") as counter_file:
            counter_file.write(str(asset_id + 1))
        release["assets"].append({
            "id": asset_id, "name": name, "size": size, "uuid": f"00000000-0000-0000-0000-{asset_id:012d}",
            "type": "attachment",
            "browser_download_url": f"https://forge.example/forgejo/dalton/fjx/releases/download/fjx%2Fv1.2.10/{name}",
        })
        write_json(record, release)
        sys.stdout.write("201")
        return 0
    if method == "DELETE" and re.search(r"/releases/42/assets/[0-9]+$", url):
        release = read_json(record)
        if not release["draft"]:
            raise Stop("public release asset deleted")
        if os.path.exists(state_path("tag")):
            raise Stop("draft asset deleted after the tag push")
        asset_id = int(os.path.basename(url))
        matches = [asset for asset in release["assets"] if asset["id"] == asset_id]
        if not matches:
            sys.stdout.write("404")
            return 0
        os.remove(os.path.join(assets, matches[0]["name"]))
        release["assets"] = [asset for asset in release["assets"] if asset["id"] != asset_id]
        write_json(record, release)
        sys.stdout.write("204")
        return 0
    if method == "PATCH":
        with open(argument(args, "--data-binary")[1:], encoding="utf-8") as body_file:
            body = json.load(body_file)
        if body != {"draft": False} or body["draft"] is not False:
            raise Stop("unexpected publication request")
        make_public(record)
        write_json(output, read_json(record))
        sys.stdout.write("503" if os.environ.get("FAKE_AMBIGUOUS_PUBLISH", "") == "true" else "200")
        return 0
    raise Stop("unexpected HTTP call")
