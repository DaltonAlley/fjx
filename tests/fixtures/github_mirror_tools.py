"""Simulated git and curl for scripts/github_mirror.py.

State lives in FAKE_MIRROR_STATE: `calls` (JSON lines of {method, url}),
`source.json` (the Forgejo release), `github.json` (the GitHub release),
`source-assets/`, `github-assets/` and `tag-lookups`. A refused call writes its reason to `mock-error`.

FAKE_STRICT_CURL=true also makes curl refuse a call that lacks the HTTPS-only
and no-redirect flags or that inherits a credential in its environment.
"""

import json
import os
import shutil
import sys
import urllib.parse

TOKEN_VARIABLES = ("FJX_TOKEN", "FORGEJO_TOKEN", "GITEA_TOKEN", "GITHUB_TOKEN", "ACTIONS_RUNTIME_TOKEN",
                   "GH_RELEASE_MIRROR_TOKEN")
GITHUB_HOSTS = ("api.github.com", "uploads.github.com")
CDN_HOST = "release-assets.githubusercontent.com"
FORGEJO_PAGE_LIMIT = 50


class Stop(Exception):
    pass


def state_path(*parts):
    return os.path.join(os.environ["FAKE_MIRROR_STATE"], *parts)


def run(function, args):
    try:
        return function(args)
    except Stop as error:
        with open(state_path("mock-error"), "w", encoding="utf-8") as file:
            file.write(f"{error}\n")
        sys.stderr.write(f"{error}\n")
        return 1


def argument(args, name):
    return args[args.index(name) + 1]


def read_json(path):
    with open(path, encoding="utf-8") as file:
        return json.load(file)


def write_json(path, value):
    with open(path, "w", encoding="utf-8") as file:
        json.dump(value, file, indent=2)


def respond(args, status, body):
    with open(argument(args, "--output"), "w", encoding="utf-8") as file:
        json.dump(body, file, separators=(",", ":"))
    sys.stdout.write(str(status))
    return 0


def git(args):
    return run(_git, args)


def _git(args):
    tag = args[-1].replace("^{}", "", 1)
    joined = " ".join(args)
    # FAKE_TAG_LATE=N: the native push mirror sends the tag to GitHub only after
    # N GitHub tag lookups.
    if "github.com" in joined and os.environ.get("FAKE_TAG_LATE", ""):
        counter = state_path("tag-lookups")
        seen = 0
        if os.path.exists(counter):
            with open(counter, encoding="utf-8") as file:
                seen = int(file.read())
        with open(counter, "w", encoding="utf-8") as file:
            file.write(str(seen + 1))
        if seen < int(os.environ["FAKE_TAG_LATE"]):
            return 0
    mismatch = os.environ.get("FAKE_TAG_MISMATCH", "") == "true" and "github.com" in joined
    print(f"{'bbbb' if mismatch else 'aaaa'}\t{tag}")
    return 0


def check_strict(args):
    if os.environ.get("FAKE_STRICT_CURL", "") != "true":
        return
    if args[:1] != ["--disable"]:
        raise Stop("unsafe curl call: --disable must come first")
    for flags in (["--proto", "=https"], ["--max-redirs", "0"], ["--config", "-"]):
        if not any(args[index:index + 2] == flags for index in range(len(args))):
            raise Stop(f"unsafe curl call: missing {' '.join(flags)}")
    if "--location" in args or "-L" in args:
        raise Stop("unsafe curl call: follows redirects")
    leaked = [name for name in TOKEN_VARIABLES if name in os.environ]
    if leaked:
        raise Stop(f"unsafe curl call: credential in environment {leaked}")


def curl(args):
    return run(_curl, args)


def _curl(args):
    configuration = sys.stdin.buffer.read().decode("utf-8")
    check_strict(args)
    url = args[-1]
    method = argument(args, "--request")
    parsed = urllib.parse.urlsplit(url)
    github = parsed.hostname in GITHUB_HOSTS
    cdn = parsed.hostname == CDN_HOST
    secret = "" if cdn else ("github-fixture-secret" if github else "forgejo-fixture-secret")
    if "fixture-secret" in " ".join(args):
        raise Stop("token in argv")
    if cdn and configuration != "":
        raise Stop("credential forwarded to CDN")
    if not cdn and secret not in configuration:
        raise Stop("wrong origin credential")
    with open(state_path("calls"), "a", encoding="utf-8") as file:
        file.write(json.dumps({"method": method, "url": url}) + "\n")
    with open(argument(args, "--dump-header"), "w", encoding="utf-8"):
        pass
    source = read_json(state_path("source.json"))
    state = state_path("github.json")
    name = os.path.basename(parsed.path)
    if cdn:
        shutil.copy(state_path("github-assets", name), argument(args, "--output"))
        sys.stdout.write("200")
        return 0
    if not github:
        if "/releases/download/" in parsed.path:
            shutil.copy(state_path("source-assets", name), argument(args, "--output"))
            sys.stdout.write("200")
            return 0
        if parsed.path.endswith("/releases/47"):
            # Download counters change while copying; they are not content identity.
            current = {**source, "assets": [{**asset, "download_count": 99} for asset in source["assets"]]}
            return respond(args, 200, current)
        # Like Forgejo: pages hold at most 50 (max_response_items), whatever
        # the requested limit. FAKE_PAGINATE puts 100 newer drafts first, so
        # the public release is on page 3.
        releases = [source]
        if os.environ.get("FAKE_PAGINATE", "") == "true":
            releases = [{**source, "id": 1000 + index, "draft": True} for index in range(100)] + [source]
        query = urllib.parse.parse_qs(parsed.query)
        size = min(int(query.get("limit", ["50"])[0]), FORGEJO_PAGE_LIMIT)
        page = int(query.get("page", ["1"])[0])
        return respond(args, 200, releases[(page - 1) * size:page * size])
    if parsed.path.endswith("/releases") and method == "GET":
        return respond(args, 200, [read_json(state)] if os.path.exists(state) else [])
    if method == "POST" and parsed.hostname == "api.github.com":
        with open(argument(args, "--data-binary")[1:], encoding="utf-8") as file:
            body = json.load(file)
        release = {**body, "id": 88, "assets": []}
        write_json(state, release)
        return respond(args, 201, release)
    if method == "POST":
        release = read_json(state)
        asset_name = parsed.query.replace("name=", "", 1)
        upload = argument(args, "--data-binary")[1:]
        shutil.copy(upload, state_path("github-assets", asset_name))
        asset = {"id": 1000 + len(release["assets"]), "name": asset_name, "size": os.path.getsize(upload)}
        release["assets"].append(asset)
        write_json(state, release)
        if os.environ.get("FAKE_UPLOAD_LOST", "") == "true":
            return 7
        return respond(args, 201, asset)
    release = read_json(state)
    if method == "DELETE" and "/releases/assets/" in parsed.path:
        if not release["draft"]:
            raise Stop("public GitHub asset deleted")
        asset_id = int(name)
        release["assets"] = [asset for asset in release["assets"] if asset["id"] != asset_id]
        write_json(state, release)
        sys.stdout.write("204")
        return 0
    if "/releases/assets/" in parsed.path:
        asset_id = int(name)
        asset = next(asset for asset in release["assets"] if asset["id"] == asset_id)
        host = "evil.example" if os.environ.get("FAKE_FOREIGN_CDN", "") == "true" else CDN_HOST
        with open(argument(args, "--dump-header"), "w", encoding="utf-8", newline="") as file:
            file.write(f"HTTP/1.1 302 Found\r\nLocation: https://{host}/{asset['name']}\r\n\r\n")
        return respond(args, 302, {})
    if parsed.path.endswith("/assets"):
        return respond(args, 200, release["assets"])
    if method == "PATCH":
        with open(argument(args, "--data-binary")[1:], encoding="utf-8") as file:
            body = json.load(file)
        updated = {**release, **body}
        write_json(state, updated)
        return respond(args, 200, updated)
    return respond(args, 200, release)
