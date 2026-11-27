"""Small fake programs for tests/test_release.py.

tests/script_support.py installs each function as an executable on PATH, for
example `curl` -> origin_curl. Fakes log what they receive as JSON lines and
never log a credential.
"""

import hashlib
import json
import os
import re
import shutil
import sys

DEFAULT_GIT_URL = "https://forge.example/dalton/monolith.git"
SCOPED_HEADER_KEY = re.compile(r"http\.https://forge\.example/(forgejo/)?\.extraHeader")


def append_json(path, value):
    with open(path, "a", encoding="utf-8") as file:
        file.write(json.dumps(value) + "\n")


def argument(args, flag):
    return args[args.index(flag) + 1]


def stop(message, code=1):
    sys.stderr.write(message + "\n")
    return code


def read_stdin():
    return b"" if sys.stdin is None or sys.stdin.closed else sys.stdin.buffer.read()


def secrets_in_env():
    return sorted(name for name in ("FJX_TOKEN", "FORGEJO_TOKEN", "GITEA_TOKEN", "GITHUB_TOKEN",
                                    "ACTIONS_RUNTIME_TOKEN", "GH_RELEASE_MIRROR_TOKEN") if name in os.environ)


def expected_header():
    token = os.environ.get("FAKE_EXPECTED_TOKEN", "")
    return ("header = " + json.dumps(f"Authorization: token {token}") + "\n").encode() if token else None


def scoped_git(args):
    """git with a remote origin, a scoped auth check for ls-remote, and fixed tags."""
    log = os.environ.get("FAKE_GIT_LOG")
    if log:
        token = os.environ.get("FAKE_EXPECTED_TOKEN", "")
        import base64
        credential = base64.b64encode(f"x-access-token:{token}".encode()).decode()
        append_json(log, {
            "args": args,
            "key_0": os.environ.get("GIT_CONFIG_KEY_0"),
            "value_0_ok": os.environ.get("GIT_CONFIG_VALUE_0") == f"Authorization: Basic {credential}",
            "key_1": os.environ.get("GIT_CONFIG_KEY_1"),
            "value_1": os.environ.get("GIT_CONFIG_VALUE_1"),
            "count": os.environ.get("GIT_CONFIG_COUNT"),
            "secrets_in_env": secrets_in_env(),
        })
    if args[:3] == ["remote", "get-url", "origin"]:
        print(os.environ.get("FAKE_GIT_URL", DEFAULT_GIT_URL))
        return 0
    if args and args[0] == "ls-remote" and not SCOPED_HEADER_KEY.fullmatch(os.environ.get("GIT_CONFIG_KEY_0", "")):
        return stop("missing scoped Git authentication")
    if args and args[0] == "rev-parse":
        print(os.environ["FAKE_REVISION"])
        return 0
    print("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb refs/tags/fjx/v1.2.3")
    target = os.environ.get("FAKE_TAG_TARGET", os.environ["FAKE_REVISION"])
    print(f"{target} refs/tags/fjx/v1.2.3^{{}}")
    extra = os.environ.get("FAKE_EXTRA_TAGS", "")
    if extra:
        print(extra)
    return 0


def log_curl(args, header):
    log = os.environ.get("FAKE_CURL_LOG")
    if log:
        expected = expected_header()
        append_json(log, {
            "args": args,
            "stdin_ok": header == expected if expected is not None else None,
            "secrets_in_env": secrets_in_env(),
        })


def metadata_curl(args):
    """Returns one public release whose metadata FAKE_METADATA changes."""
    log_curl(args, read_stdin())
    output = argument(args, "--output")
    state = os.environ["FAKE_METADATA"]
    names = json.loads(os.environ["FAKE_ASSET_NAMES"])
    release = {
        "id": 42,
        "tag_name": "fjx/v9.9.9" if state == "tag" else "fjx/v1.2.3",
        "name": "wrong" if state == "title" else "fjx (1.2.3)",
        "body": "wrong" if state == "body" else "",
        "draft": state == "draft",
        "prerelease": state == "prerelease",
        "assets": [{"name": name} for name in names],
    }
    if state == "draft-number":
        release["draft"] = 0
    with open(output, "w", encoding="utf-8") as file:
        json.dump(release, file)
    sys.stdout.write("200")
    return 0


def origin_curl(args):
    """Serves a public release whose asset URLs FAKE_RELEASE_STATE makes unsafe."""
    header = read_stdin()
    log_curl(args, header)
    output = argument(args, "--output")
    target = args[-1]
    state = os.environ["FAKE_RELEASE_STATE"]
    if "evil.example" in target and b"Authorization" in header:
        with open(os.environ["FAKE_FOREIGN_LOG"], "a", encoding="utf-8") as file:
            file.write("token disclosed")
    if re.search(r"/releases/download/fjx%2Fv1[.]2[.]3/", target):
        if state == "redirect":
            with open(output, "w", encoding="utf-8") as file:
                file.write("redirect")
            sys.stdout.write("302")
            return 0
        source = os.path.join(os.environ["FAKE_ASSET_ROOT"], os.path.basename(target))
        if state == "corrupt" and source.endswith(".tar.gz"):
            with open(source, "rb") as file:
                data = file.read()
            with open(output, "wb") as file:
                file.write(data[::-1])
        else:
            shutil.copyfile(source, output)
        sys.stdout.write("200")
        return 0
    names = json.loads(os.environ["FAKE_ASSET_NAMES"])
    origin = {"foreign": "https://evil.example", "http-asset": "http://forge.example"}.get(
        state, os.environ["FORGEJO_API_URL"].replace("/api/v1", "", 1))
    repository = "dalton/other" if state == "foreign-path" else "dalton/monolith"
    assets = []
    for index, name in enumerate(names):
        asset = {"name": name, "browser_download_url": f"{origin}/{repository}/releases/download/fjx%2Fv1.2.3/{name}"}
        if not (state == "missing-id" and index == 0):
            asset["id"] = 0 if state == "invalid-id" and index == 0 else index + 101
        if state == "bool-id" and index == 0:
            asset["id"] = True
        assets.append(asset)
    release = {"id": 42, "tag_name": "fjx/v1.2.3", "name": "fjx (1.2.3)", "body": "", "draft": False,
               "prerelease": False, "assets": assets}
    with open(output, "w", encoding="utf-8") as file:
        json.dump(release, file)
    sys.stdout.write("200")
    return 0


def create_curl(args):
    """Accepts exactly one draft creation request for dalton/fjx below /forgejo."""
    with open(os.environ["FAKE_CREATE_CALLS"], "a", encoding="utf-8") as file:
        file.write("request\n")
    header = read_stdin()
    if b"Authorization: token fixture-create" not in header:
        return 9
    if any("fixture-create" in arg for arg in args):
        return 9
    if args[-1] != "https://forge.example/forgejo/api/v1/repos/dalton/fjx/releases":
        return 9
    with open(argument(args, "--data-binary")[1:], "rb") as file:
        body = json.load(file)
    if body != {"tag_name": "fjx/v1.2.3", "name": "fjx (1.2.3)", "body": "", "draft": True, "prerelease": False} \
            or body["draft"] is not True or body["prerelease"] is not False:
        return 9
    body["id"] = 42
    with open(argument(args, "--output"), "w", encoding="utf-8") as file:
        json.dump(body, file)
    sys.stdout.write(os.environ["FAKE_CREATE_STATUS"])
    return 0


def build_runtime(args):
    """Docker or Podman build: logs argv and writes the target's executable."""
    append_json(os.environ["FAKE_BUILD_LOG"], args)

    def value(prefix):
        return next(arg for arg in args if arg.startswith(prefix))[len(prefix):]

    target = value("TARGET=")
    executable = value("EXECUTABLE=")
    destination = value("type=local,dest=")
    if os.environ.get("FAKE_BUILD_FAIL_TARGET", "") == target:
        return 17
    os.makedirs(destination, exist_ok=True)
    with open(os.path.join(destination, executable), "w", encoding="utf-8") as file:
        file.write(f"#!/bin/sh\necho {target}\n")
    return 0


def smoke_runtime(args):
    """Container runtime for the ARM64 smoke state machine; FAKE_SMOKE_STATES feeds inspect."""
    command, arguments = args[0], args[1:]
    append_json(os.environ["FAKE_SMOKE_CALLS"], {"command": command, "arguments": arguments})
    if command == "cp":
        if os.environ.get("FAKE_SMOKE_FAIL_COPY", "") == os.path.basename(arguments[0]):
            return 12
        return 0
    if command == "start":
        print("fake-smoke-container")
        return 0
    if command == "inspect":
        with open(os.environ["FAKE_SMOKE_STATES"], encoding="utf-8") as file:
            states = file.read().split("\n")
        index_path = os.environ["FAKE_SMOKE_INDEX"]
        with open(index_path, encoding="utf-8") as file:
            index = int(file.read())
        with open(index_path, "w", encoding="utf-8") as file:
            file.write(str(index + 1))
        sys.stdout.write(states[min(index, len(states) - 1)])
        return 0
    if command == "logs":
        print("fake-fjx-help")
        return 0
    if command == "rm":
        return 0
    return stop(f"unexpected fake runtime command {command}")


FAKE_QEMU = ("#!/bin/sh\n# format: ELF 64-bit LSB executable, x86-64, version 1 (SYSV), statically linked\n"
             "echo 'qemu-aarch64 version 10.2.3 (fake)'\n")


def verify_runtime(args):
    """Container runtime for `verify --smoke`: logs argv, hashes `run` input, serves a fake QEMU."""
    entry = {"args": args, "secrets_in_env": secrets_in_env()}
    if args and args[0] == "run":
        entry["stdin_sha256"] = hashlib.sha256(read_stdin()).hexdigest()
    append_json(os.environ["FAKE_RUNTIME_LOG"], entry)
    command = args[0]
    if command == "create":
        print("qemu-container" if args[1:3] == ["--platform", "linux/amd64"] else "smoke-container")
    elif command == "cp" and args[1].startswith("qemu-container:"):
        with open(args[2], "w", encoding="utf-8") as file:
            file.write(FAKE_QEMU)
    elif command == "inspect":
        sys.stdout.write(os.environ.get("FAKE_SMOKE_STATE", '[{"State":{"Status":"exited","ExitCode":0}}]'))
    elif command == "logs":
        print("fake-fjx-help")
    elif command == "run" and os.environ.get("FAKE_RUN_FAIL", "") == "true":
        return 3
    return 0


def wine(args):
    append_json(os.environ["FAKE_RUNTIME_LOG"], {"args": ["wine", *[os.path.basename(arg) for arg in args]],
                                                 "secrets_in_env": secrets_in_env()})
    return 0


def fake_file(args):
    """`file --brief PATH`: prints the `# format: ` line embedded in the fixture binary."""
    with open(args[-1], "rb") as file:
        for line in file.read().split(b"\n"):
            if line.startswith(b"# format: "):
                print(line[len(b"# format: "):].decode())
                return 0
    print("data")
    return 0
