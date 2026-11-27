"""Tests for scripts/github_mirror.py against the fakes in fixtures/github_mirror_tools.py."""

from __future__ import annotations

import hashlib
import json
import os
import re
import subprocess
import time

from script_support import TempTestCase, install_tool, read, run_script, write

import github_mirror

TAG = "fjx/v1.2.3"
SOURCE_URL = "https://dyad.tail77425.ts.net/forgejo/dalton/fjx/releases/download/fjx%2Fv1.2.3"


class MirrorTests(TempTestCase):
    def setUp(self):
        super().setUp()
        bin_dir = self.mkdir("bin")
        self.assets = self.mkdir("source-assets")
        self.mkdir("github-assets")
        self.state = self.path("github.json")
        write(self.path("calls"), "")
        for name in ("curl", "git"):
            install_tool(bin_dir, name, "github_mirror_tools", name)
        names = github_mirror.asset_names(TAG)
        for name in [name for name in names if not name.endswith(".sha256")]:
            data = f"fixture bytes for {name}".encode()
            write(os.path.join(self.assets, name), data)
            write(os.path.join(self.assets, f"{name}.sha256"), f"{hashlib.sha256(data).hexdigest()}  {name}\n")
        self.release = {
            "id": 47, "tag_name": TAG, "name": "A release", "body": "Notes\nwith newlines", "draft": False,
            "prerelease": False,
            "assets": [{"id": index + 1, "name": name, "size": os.path.getsize(os.path.join(self.assets, name)),
                        "browser_download_url": f"{SOURCE_URL}/{name}"} for index, name in enumerate(names)],
        }
        self.set_source(self.release)
        self.env = {
            "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
            "FAKE_MIRROR_STATE": self.tmp,
            "FAKE_STRICT_CURL": "true",
            "GH_RELEASE_MIRROR_TOKEN": "github-fixture-secret",
            "FORGEJO_TOKEN": "forgejo-fixture-secret",
            "FJX_MIRROR_TAG_WAIT_SECONDS": "2",
        }
        self.results: list[subprocess.CompletedProcess] = []

    def set_source(self, release: dict):
        write(self.path("source.json"), json.dumps(release))

    def github(self) -> dict:
        with open(self.state, encoding="utf-8") as file:
            return json.load(file)

    def set_github(self, release: dict):
        write(self.state, json.dumps(release))

    def run_mirror(self, **extra: str) -> subprocess.CompletedProcess:
        result = run_script("github_mirror.py", [], env={**self.env, **extra})
        self.results.append(result)
        self.assertNotIn("Traceback", result.stderr)
        return result

    def calls(self) -> list[dict]:
        with open(self.path("calls"), encoding="utf-8") as file:
            return [json.loads(line) for line in file if line.strip()]

    def writes(self) -> list[dict]:
        return [call for call in self.calls() if call["method"] != "GET"]

    def reset_calls(self):
        write(self.path("calls"), "")

    def assertStopsBeforeWrites(self, message: str, error: str | None = None, **extra: str):
        result = self.run_mirror(**extra)
        self.assertFailed(result, error)
        self.assertEqual(self.writes(), [], message)
        return result

    def assertNoSecrets(self):
        self.assertNotIn("fixture-secret", read(self.path("calls")).decode(), "credentials appeared in request logs")
        for result in self.results:
            self.assertNotIn("fixture-secret", result.stdout + result.stderr, "credentials appeared in output")
        self.assertFalse(os.path.exists(self.path("mock-error")), read(self.path("mock-error")).decode()
                         if os.path.exists(self.path("mock-error")) else "")

    def test_mirror_flow(self):
        self.set_source({**self.release, "draft": True})
        skipped = self.run_mirror()
        self.assertSucceeded(skipped)
        self.assertEqual(self.writes(), [], "source drafts must never be published")
        self.set_source({**self.release, "assets": self.release["assets"][1:]})
        self.assertStopsBeforeWrites("incomplete source release must fail before writes",
                                     "published source release does not have all twelve assets")
        self.set_source(self.release)
        self.assertStopsBeforeWrites("mismatched tags must stop copying", "release tag hashes differ",
                                     FAKE_TAG_MISMATCH="true")
        started = time.monotonic()
        self.assertStopsBeforeWrites("a tag that never reaches GitHub must fail before writes",
                                     "release tag hashes differ", FAKE_TAG_LATE="1000")
        self.assertGreaterEqual(time.monotonic() - started, 2, "a missing GitHub tag must be waited for before failing")
        os.remove(self.path("tag-lookups"))
        bad = json.loads(json.dumps(self.release))
        bad["assets"][0]["browser_download_url"] = "https://evil.example/asset"
        self.set_source(bad)
        self.assertStopsBeforeWrites("foreign source asset URL must fail before writes",
                                     "source asset URL must identify this exact repository")
        self.set_source(self.release)
        checksum = os.path.join(self.assets, self.release["assets"][1]["name"])
        original = read(checksum)
        write(checksum, "bad checksum")
        self.assertStopsBeforeWrites("a resized checksum file must fail before writes",
                                     "source asset size changed during download")
        write(checksum, ("0" if original[:1] != b"0" else "1").encode() + original[1:])
        self.assertStopsBeforeWrites("bad checksums must fail before writes",
                                     "source archive checksum does not match its checksum file")
        write(checksum, original)

        interrupted = self.run_mirror(FAKE_UPLOAD_LOST="true")
        self.assertFailed(interrupted, "HTTP transfer failed")
        self.assertTrue(self.github()["draft"])
        self.assertEqual(len(self.github()["assets"]), 1, "lost response must preserve the partial draft")
        self.assertEqual(len(self.writes()), 2, "ambiguous upload must not be retried in the same run")
        self.reset_calls()
        # The partial draft holds stale bytes. The rerun must replace them.
        stale = self.github()["assets"][0]["name"]
        write(self.path("github-assets", stale), "stale bytes")
        # The release job copies right after publishing; the tag may arrive late.
        recovered = self.run_mirror(FAKE_PAGINATE="true", FAKE_TAG_LATE="1")
        self.assertSucceeded(recovered)
        self.assertEqual(recovered.stdout,
                         f"Verified {TAG}: notes, prerelease state, tag hashes, and twelve identical assets.\n")
        self.assertGreaterEqual(int(read(self.path("tag-lookups"))), 2, "late GitHub tag was not looked up again")
        published = self.github()
        self.assertFalse(published["draft"])
        self.assertEqual(len(published["assets"]), 12)
        self.assertEqual(published["body"], self.release["body"], "rerun must finish and publish the same release")
        self.assertEqual(published["make_latest"], "legacy")
        self.assertEqual(len([call for call in self.writes() if call["method"] == "DELETE"]), 1,
                         "rerun must delete the stale draft asset")
        self.assertEqual(len([call for call in self.writes() if call["method"] == "POST"]), 12,
                         "rerun must upload all twelve assets to the draft")
        self.assertEqual(read(self.path("github-assets", stale)), read(os.path.join(self.assets, stale)),
                         "stale draft bytes were published")
        self.reset_calls()
        repeated = self.run_mirror()
        self.assertSucceeded(repeated)
        self.assertEqual(self.writes(), [], "complete rerun must verify bytes without writes")

        self.set_source({**self.release, "body": "Updated notes", "prerelease": True})
        edited = self.run_mirror()
        self.assertSucceeded(edited)
        state = self.github()
        self.assertTrue(state["prerelease"])
        self.assertEqual(state["make_latest"], "false")
        self.assertEqual(state["body"], "Updated notes", "prerelease edit must preserve state and never become latest")
        self.reset_calls()
        self.assertStopsBeforeWrites("foreign CDN must fail without writes",
                                     "GitHub asset redirect must use the GitHub release CDN", FAKE_FOREIGN_CDN="true")
        self.assertTrue(all("evil.example" not in call["url"] for call in self.calls()), "foreign CDN was contacted")
        first = self.release["assets"][0]["name"]
        write(self.path("github-assets", first), "corruption")
        self.assertStopsBeforeWrites("conflicting bytes must fail without overwrite", "GitHub asset differs from Forgejo")
        write(self.path("github-assets", first), read(os.path.join(self.assets, first)))
        self.set_github({**self.github(), "assets": self.github()["assets"][1:]})
        self.assertStopsBeforeWrites("partial public release needs explicit repair",
                                     "existing public GitHub release is incomplete")
        self.assertNoSecrets()

    def test_forgejo_pages_are_read_until_empty(self):
        # The fake caps pages at 50 like Forgejo. 100 drafts fill pages 1-2, so
        # the one public release is on page 3. Stopping at the first page
        # shorter than the requested 100 would never see it.
        result = self.run_mirror(FAKE_PAGINATE="true")
        self.assertSucceeded(result)
        self.assertIn(f"Verified {TAG}", result.stdout)
        self.assertFalse(self.github()["draft"], "the release on page 3 was not mirrored")
        listing = [call["url"] for call in self.calls()
                   if call["url"].startswith("https://dyad.tail77425.ts.net/forgejo/api/v1/repos/dalton/fjx/releases?")]
        self.assertEqual([re.search(r"[?&]page=([0-9]+)", url).group(1) for url in listing], ["1", "2", "3", "4"],
                         "Forgejo pages must be read until an empty page")

    def test_source_records_need_exact_types(self):
        # Each case would be read loosely by a careless port.
        cases = {
            "draft is not a boolean": ({**self.release, "draft": "false"},
                                       "release must declare draft and prerelease state"),
            "ID is not an integer": ({**self.release, "id": 47.0}, "release has no valid ID"),
            "prerelease tag not marked": ({**self.release, "tag_name": "fjx/v1.2.3-rc.1"},
                                          "prerelease version must be marked as a prerelease"),
            "tag is not fjx/vVERSION": ({**self.release, "tag_name": "fjx/v1.2"}, "release tag must use fjx/vVERSION"),
            "notes are not text": ({**self.release, "body": ["Notes"]}, "release body must be text"),
            "assets are not a list": ({**self.release, "assets": {}}, "release assets must be a list"),
        }
        asset_cases = {
            "size is text": ("size", "5", "asset has an invalid ID or size"),
            "ID is a boolean": ("id", True, "asset has an invalid ID or size"),
            "name is not text": ("name", 7, "duplicate or unexpected release assets"),
        }
        for label, (key, value, error) in asset_cases.items():
            release = json.loads(json.dumps(self.release))
            release["assets"][2][key] = value
            cases[label] = (release, error)
        duplicate = json.loads(json.dumps(self.release))
        duplicate["assets"][1] = {**duplicate["assets"][0], "id": 99}
        cases["duplicate name"] = (duplicate, "duplicate or unexpected release assets")
        for label, (release, error) in cases.items():
            with self.subTest(label):
                self.set_source(release)
                self.reset_calls()
                self.assertStopsBeforeWrites(f"{label} must fail before writes", error)
        self.set_source(self.release)
        # The fake wraps source.json in the listing array.
        for listing, error in (({"id": 47}, "release must declare draft and prerelease state"),
                               ([3], "expected a paginated array of objects")):
            with self.subTest(listing=listing):
                write(self.path("source.json"), json.dumps(listing))
                self.assertFailed(run_script("github_mirror.py", [], env=self.env), error)

    def test_github_records_need_exact_types(self):
        self.assertSucceeded(self.run_mirror())
        published = self.github()
        for label, change, error in (
                ("release ID is a float", {"id": 88.0}, "invalid GitHub release identity"),
                ("draft is missing", {"draft": None}, "invalid GitHub release identity"),
                ("asset size is text", {"assets": [{**published["assets"][0], "size": "1"}, *published["assets"][1:]]},
                 "asset has an invalid ID or size")):
            with self.subTest(label):
                self.set_github({**published, **change})
                self.reset_calls()
                self.assertStopsBeforeWrites(f"{label} must fail before writes", error)

    def test_credentials_stay_in_scope(self):
        for name, error in (("GH_RELEASE_MIRROR_TOKEN", "required mirror credential is missing"),
                            ("FORGEJO_TOKEN", "required mirror credential is missing")):
            with self.subTest(name):
                self.assertFailed(run_script("github_mirror.py", [], env={**self.env, name: " "}), error)
        self.assertFailed(run_script("github_mirror.py", [], env={**self.env, "FORGEJO_TOKEN": "a\nb"}),
                          "invalid mirror credential")
        # The CDN request gets no credential and curl never sees a token in its
        # environment (FAKE_STRICT_CURL checks both).
        self.assertSucceeded(self.run_mirror())
        self.assertNoSecrets()

    def test_requests_stay_on_https_services(self):
        directory = self.mkdir("requests")
        for url, error in (("http://api.github.com/x", "request must use HTTPS without user information"),
                           ("https://user@api.github.com/x", "request must use HTTPS without user information"),
                           ("https://api.github.com\\@evil.example/", "request must use HTTPS without user information"),
                           ("https://evil.example/x", "refusing authentication outside the configured services"),
                           ("https://dyad.tail77425.ts.net/forgejoevil/x",
                            "refusing authentication outside the configured services")):
            with self.subTest(url):
                with self.assertRaises(github_mirror.Failure) as caught:
                    github_mirror.request("GET", url, directory)
                self.assertIn(error, str(caught.exception))

    def test_usage(self):
        result = run_script("github_mirror.py", ["extra"])
        self.assertEqual(result.returncode, 1)
        self.assertNotIn("Traceback", result.stderr)
