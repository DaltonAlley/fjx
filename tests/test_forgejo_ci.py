"""Tests for scripts/forgejo_ci.py: release selection and the publication flow.

The publication flow runs the real scripts against the fake git, cargo, curl
and file in fixtures/forgejo_release_tools.py.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
from unittest import mock

from script_support import ROOT, SCRIPTS, TempTestCase, base_env, install_tool, read, run_script, write

import forgejo_ci
from release import Failure

SCRIPT_FILES = ("forgejo_ci.py", "release.py", "package_release.py", "release_smoke.py")
WRITES = ("POST", "PATCH", "push-tag", "DELETE")
FIRST_ASSET = "fjx-1.2.10-aarch64-apple-darwin.tar.gz"
WINDOWS_ZIP = "fjx-1.2.10-x86_64-pc-windows-gnu.zip"
TAG = "fjx/v1.2.10"


def git(project: str, *arguments: str) -> str:
    result = subprocess.run(["git", *arguments], cwd=project, capture_output=True, text=True,
                            env=base_env({"GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1"}))
    if result.returncode != 0:
        raise AssertionError(result.stderr)
    return result.stdout.strip()


def commit_version(project: str, version: str) -> str:
    write(os.path.join(project, "release.toml"), f"version = '{version}'\n")
    write(os.path.join(project, "Cargo.toml"), f"[package]\nname = 'fjx'\nversion = '{version}'\n")
    write(os.path.join(project, "Cargo.lock"), f"version = 4\n[[package]]\nname = 'fjx'\nversion = '{version}'\n")
    git(project, "add", ".")
    git(project, "-c", "user.name=Fixture", "-c", "user.email=fixture@localhost", "commit", "--allow-empty",
        "-m", version)
    return git(project, "rev-parse", "HEAD")


class CandidateTests(TempTestCase):
    def setUp(self):
        super().setUp()
        self.project = self.mkdir("fjx")
        previous = os.getcwd()
        os.chdir(self.project)
        self.addCleanup(os.chdir, previous)
        patcher = mock.patch.dict(os.environ, {"RELEASE_WORKSPACE_ROOT": self.tmp})
        patcher.start()
        self.addCleanup(patcher.stop)
        git(self.project, "init")

    def test_release_candidates(self):
        candidate = forgejo_ci.release_candidate
        before = commit_version(self.project, "1.2.9")
        unchanged = commit_version(self.project, "1.2.9")
        self.assertFalse(candidate(before, unchanged), "ordinary commits must not publish")
        increased = commit_version(self.project, "1.2.10")
        self.assertTrue(candidate(before, increased), "numeric patch increases must publish")
        self.assertFalse(candidate("0" * 40, increased), "initial imports must not publish")
        for arguments in ([before, unchanged], ["bad", increased], [increased.replace("a", "z"), "bad"]):
            with self.assertRaises(Failure, msg="wrong or invalid revisions must fail"):
                candidate(*arguments)
        with self.assertRaises(Failure, msg="a before revision that is not hex must fail") as caught:
            candidate("z" * 40, increased)
        self.assertIn("push has no valid before revision", str(caught.exception))
        decreased = commit_version(self.project, "1.2.8")
        with self.assertRaises(Failure, msg="version decreases must fail") as caught:
            candidate(increased, decreased)
        self.assertIn("release version must increase", str(caught.exception))
        major = commit_version(self.project, "2.0.0")
        self.assertTrue(candidate(decreased, major), "major increases must publish")
        write(os.path.join(self.project, "release.toml"), "version = '9.9.9'\n")
        with self.assertRaises(Failure, msg="inconsistent version files must fail"):
            candidate(decreased, major)

    def test_previous_version_must_be_valid(self):
        before = commit_version(self.project, "1.2.9")
        write(os.path.join(self.project, "release.toml"), "name = 'no version'\n")
        git(self.project, "-c", "user.name=Fixture", "-c", "user.email=fixture@localhost", "commit", "-am", "x")
        broken = git(self.project, "rev-parse", "HEAD")
        revision = commit_version(self.project, "1.2.10")
        with self.assertRaises(Failure) as caught:
            forgejo_ci.release_candidate(broken, revision)
        self.assertIn("invalid previous release.toml", str(caught.exception))
        self.assertTrue(forgejo_ci.release_candidate(before, revision))


class PublicationFlowTests(TempTestCase):
    """One server state carried through failed runs, reruns and recoveries."""

    def setUp(self):
        super().setUp()
        self.project = self.mkdir("fjx")
        self.state = self.mkdir("server")
        self.mkdir("server", "assets")
        scripts = self.mkdir("fjx", "scripts")
        for name in SCRIPT_FILES:
            shutil.copy(os.path.join(SCRIPTS, name), scripts)
        shutil.copy(os.path.join(ROOT, "release-targets.toml"), self.project)
        self.real_git = shutil.which("git")
        bin_dir = self.mkdir("bin")
        for name in ("git", "cargo", "curl", "file"):
            install_tool(bin_dir, name, "forgejo_release_tools", name)
        write(self.path("server", "calls"), "")
        git(self.project, "init")
        self.before = commit_version(self.project, "1.2.9")
        self.unchanged = commit_version(self.project, "1.2.9")
        self.revision = commit_version(self.project, "1.2.10")
        self.env = {
            "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
            "FAKE_STATE": self.state,
            "FAKE_PROJECT": self.project,
            "FAKE_REAL_GIT": self.real_git,
            "FAKE_STRICT_CURL": "true",
            "FAKE_EXPECTED_TOKEN": "fixture",
            "CARGO_TARGET_DIR": self.path("target"),
            "FORGEJO_API_URL": "https://forge.example/forgejo/api/v1",
            "FORGEJO_REPOSITORY": "dalton/fjx",
            "FORGEJO_TOKEN": "fixture",
            "FORGEJO_EVENT_NAME": "push",
            "FORGEJO_REF": "refs/heads/main",
            "BEFORE_SHA": self.before,
        }

    # Helpers -------------------------------------------------------------

    def run_ci(self, *arguments: str, **extra: str) -> subprocess.CompletedProcess:
        script = os.path.join(self.project, "scripts", "forgejo_ci.py")
        return run_script(script, list(arguments), env={**self.env, **extra}, cwd=self.project)

    def calls(self) -> list[dict]:
        with open(os.path.join(self.state, "calls"), encoding="utf-8") as file:
            return [json.loads(line) for line in file if line.strip()]

    def events(self, name: str) -> list[dict]:
        return [call for call in self.calls() if call["event"] == name]

    def reset_calls(self):
        write(os.path.join(self.state, "calls"), "")

    def fresh_checkout(self):
        # A rerun gets a clean job container while the simulated server keeps state.
        shutil.rmtree(os.path.join(self.project, "publish-assets"), ignore_errors=True)
        self.reset_calls()

    def release(self) -> dict:
        with open(os.path.join(self.state, "release.json"), encoding="utf-8") as file:
            return json.load(file)

    def save_release(self, release: dict):
        write(os.path.join(self.state, "release.json"), json.dumps(release))

    def empty_server(self):
        for name in ("release.json", "tag", "pending-tag-sync"):
            path = os.path.join(self.state, name)
            if os.path.exists(path):
                os.remove(path)
        shutil.rmtree(os.path.join(self.state, "assets"))
        os.makedirs(os.path.join(self.state, "assets"))
        subprocess.run([self.real_git, "tag", "--delete", TAG], cwd=self.project, capture_output=True)
        self.fresh_checkout()

    def assertReadOnly(self, message: str):
        self.assertTrue(all(call["event"] not in WRITES for call in self.calls()), message)

    def index(self, name: str, last: bool = False) -> int:
        indexes = [position for position, call in enumerate(self.calls()) if call["event"] == name]
        return indexes[-1] if last else indexes[0]

    # The flow ------------------------------------------------------------

    def test_publication_flow(self):
        self.check_selection()
        self.check_failed_builds()
        self.check_failed_upload()
        self.check_wrong_draft_metadata()
        self.check_ambiguous_publication()
        self.check_published_rerun()
        self.check_clean_publications()
        self.check_stale_draft_replaced()
        self.check_tagged_draft_kept()
        self.check_late_byte_mismatch()

    def check_selection(self):
        output = self.path("output")
        candidate = self.run_ci("candidate", self.before, self.revision, output)
        self.assertSucceeded(candidate)
        self.assertEqual(read(output), b"release=true\n", "main version increase must reach the release job")
        for context in ({"FORGEJO_EVENT_NAME": "pull_request"}, {"FORGEJO_EVENT_NAME": "workflow_dispatch"},
                        {"FORGEJO_REF": "refs/heads/feature"}):
            os.remove(output)
            candidate = self.run_ci("candidate", self.before, self.revision, output, **context)
            self.assertSucceeded(candidate)
            self.assertEqual(read(output), b"release=false\n",
                             "PRs, dispatches and other branches must not select publication")
            self.assertFailed(self.run_ci("publish", self.revision, **context),
                              "publication is only allowed for a push to main")
        git(self.project, "checkout", "--detach", self.unchanged)
        os.remove(output)
        candidate = self.run_ci("candidate", self.before, self.unchanged, output)
        self.assertSucceeded(candidate)
        self.assertEqual(read(output), b"release=false\n", "unchanged versions must skip publication")
        self.assertFailed(self.run_ci("publish", self.unchanged),
                          "publication requires a version increase across this push")
        self.assertEqual(self.calls(), [], "non-candidates must not build or contact the release server")
        git(self.project, "checkout", "--detach", self.revision)
        # The output file is appended to, as GITHUB_OUTPUT requires.
        self.assertSucceeded(self.run_ci("candidate", self.before, self.revision, output))
        self.assertEqual(read(output), b"release=false\nrelease=true\n")

    def check_failed_builds(self):
        failed = self.run_ci("publish", self.revision, FAKE_FAIL_BUILD="x86_64-pc-windows-gnu")
        self.assertFailed(failed, "cargo failed; publication stopped")
        self.assertFalse(os.path.exists(os.path.join(self.state, "release.json")), "failed build created a release")
        self.assertFalse(os.path.exists(os.path.join(self.state, "tag")), "failed build pushed a tag")
        self.assertReadOnly("failed build attempted a write")

        # A Windows build whose PE timestamp is not the commit time is not
        # reproducible, so a rerun could not rebuild the same bytes.
        self.fresh_checkout()
        unpinned = self.run_ci("publish", self.revision, FAKE_UNPINNED_LINK="true")
        self.assertFailed(unpinned, "PE timestamp")
        self.assertReadOnly("unpinned build attempted a write")
        # build refuses to reuse assets left by an earlier build.
        self.assertFailed(self.run_ci("build", self.revision), "publish-assets must not already exist")
        self.fresh_checkout()
        self.assertSucceeded(self.run_ci("build", self.revision))
        self.assertEqual(len(os.listdir(os.path.join(self.project, "publish-assets"))), 12)
        self.assertEqual(len(self.events("build")), 6)
        self.assertReadOnly("build attempted a write")
        self.fresh_checkout()

    def check_failed_upload(self):
        failed_asset = f"{FIRST_ASSET}.sha256"
        failed = self.run_ci("publish", self.revision, FAKE_FAIL_UPLOAD=failed_asset)
        self.assertFailed(failed, "release.py failed; publication stopped")
        draft = self.release()
        self.assertTrue(draft["draft"])
        self.assertEqual([asset["name"] for asset in draft["assets"]], [FIRST_ASSET],
                         "failed upload must leave only a partial draft")
        calls = self.calls()
        first_write = next(index for index, call in enumerate(calls) if call["event"] in ("POST", "push-tag"))
        preparation = calls[:first_write]
        self.assertEqual(len([call for call in preparation if call["event"] == "build"]), 6,
                         "writes began before all targets built")
        self.assertEqual(len([call for call in preparation if call["event"] == "format-check"]), 6,
                         "writes began before local archive verification")
        self.assertEqual(self.events("PATCH"), [], "failed upload published a partial release")
        # Forgejo publishes a draft when it syncs a pushed tag of the same name.
        self.assertEqual(self.events("push-tag"), [], "tag was pushed before the draft was complete and verified")
        self.assertEqual(len([call for call in self.events("POST") if call["value"].endswith(failed_asset)]), 1,
                         "failed upload was retried blindly")

    def check_wrong_draft_metadata(self):
        # The job token cannot read draft bytes, so the draft is checked by its
        # API metadata. A wrong size, foreign URL or missing ID must stop before
        # the tag push and the public transition. Each rerun first replaces the
        # assets an earlier run left on the untagged draft. An asset without an
        # ID cannot be deleted, so that run stops before any write.
        for metadata, deletes, uploads in (("size", 1, 12), ("origin", 12, 12), ("id", 0, 0)):
            self.fresh_checkout()
            bad = self.run_ci("publish", self.revision, FAKE_DRAFT_METADATA=metadata)
            self.assertFailed(bad)
            self.assertTrue(self.release()["draft"], f"wrong draft {metadata} published a release")
            self.assertEqual(self.events("PATCH"), [], "wrong draft metadata reached the public transition")
            self.assertEqual(self.events("push-tag"), [],
                             "wrong draft metadata reached the tag push that publishes a draft")
            self.assertEqual(self.events("download"), [], "draft check tried to download draft bytes")
            writes = [call["event"] for call in self.calls() if call["event"] in ("DELETE", "POST")]
            self.assertEqual(writes, ["DELETE"] * deletes + ["POST"] * uploads,
                             f"rerun {metadata} must delete every old draft asset, then upload all twelve")

    def check_ambiguous_publication(self):
        self.fresh_checkout()
        ambiguous = self.run_ci("publish", self.revision, FAKE_AMBIGUOUS_PUBLISH="true")
        self.assertFailed(ambiguous, "release.py failed; publication stopped")
        posts = self.events("POST")
        self.assertEqual([call for call in posts if call["value"].endswith("/releases")], [],
                         "draft retry must not create a second release")
        self.assertEqual((len(self.events("DELETE")), len(posts)), (12, 12),
                         "untagged draft retry must replace all twelve assets")
        self.assertEqual(len(self.events("PATCH")), 1, "publication write was retried")
        publication = self.index("PATCH")
        self.assertEqual([call for call in self.calls()[:publication] if call["event"] == "download"], [],
                         "publication tried to download draft bytes")
        self.assertEqual(len(self.events("push-tag")), 1, "complete draft must push the tag once")
        self.assertEqual([call["value"] for call in self.events("public")], ["12"],
                         "release must become public once, with all twelve assets")

    def check_published_rerun(self):
        self.fresh_checkout()
        retry = self.run_ci("publish", self.revision)
        self.assertSucceeded(retry)
        self.assertEqual(retry.stdout.splitlines()[-1], f"Published and verified {TAG}.")
        published = self.release()
        self.assertFalse(published["draft"])
        self.assertEqual(len(published["assets"]), 12, "recovery must verify a complete public release")
        self.assertReadOnly("published retry must be read-only after the rebuild")
        self.assertEqual(len(self.events("download")), 12, "published retry must verify every attachment again")

        # An old public-but-partial release is an explicit repair, never appended.
        self.save_release({**published, "assets": published["assets"][:-1]})
        self.fresh_checkout()
        self.assertFailed(self.run_ci("publish", self.revision), "published release is incomplete")
        self.assertReadOnly("partial public release was modified")

    def check_clean_publications(self):
        # A normal, unambiguous publication from an empty server, with Forgejo's
        # tag sync publishing the draft (queued) or not (never). Before the fix,
        # the queued sync published the draft while assets were still uploading
        # and publish-release failed with 'metadata is not exact'.
        for sync in ("queued", "never"):
            self.empty_server()
            success = self.run_ci("publish", self.revision, FAKE_TAG_SYNC=sync)
            self.assertSucceeded(success)
            self.assertFalse(self.release()["draft"], "successful run left a draft")
            self.assertEqual(len(self.events("POST")), 13, "new release must create one draft and twelve attachments")
            self.assertEqual(len(self.events("PATCH")), 1, "successful run must publish once")
            self.assertEqual(len(self.events("download")), 12,
                             "successful run must verify all bytes once after publication")
            self.assertEqual([call["value"] for call in self.events("public")], ["12"],
                             f"{sync} tag sync: release must become public once, with all twelve assets")
            self.assertEqual(len(self.events("tag-sync")), 1 if sync == "queued" else 0,
                             "tag sync model did not run as configured")
            calls = self.calls()
            tag_push = self.index("push-tag")
            self.assertEqual([call for call in calls[:tag_push] if call["event"] == "download"], [],
                             "tag push must not wait on draft bytes the job token cannot read")
            self.assertEqual(len([call for call in calls[tag_push:] if call["event"] == "download"]), 12,
                             "public release bytes must be verified after the tag push")

    def check_stale_draft_replaced(self):
        # Issue #27: an untagged draft holds a same-size Windows zip from an
        # earlier run. The rerun must replace it, not publish the old bytes.
        self.empty_server()
        self.assertFailed(self.run_ci("publish", self.revision, FAKE_DRAFT_METADATA="size"))
        windows_zip = os.path.join(self.state, "assets", WINDOWS_ZIP)
        write(windows_zip, read(windows_zip)[::-1])
        self.fresh_checkout()
        replaced = self.run_ci("publish", self.revision)
        self.assertSucceeded(replaced)
        self.assertEqual(len(self.events("DELETE")), 12, "rerun must delete every stale draft asset")
        self.assertEqual(len(self.events("POST")), 12, "rerun must upload all twelve assets again")
        self.assertLess(self.index("DELETE", last=True), self.index("push-tag"),
                        "draft assets must be deleted before the tag push")

    def check_tagged_draft_kept(self):
        # Once the tag is pushed, Forgejo may publish the draft at any moment.
        # A rerun must keep the draft assets and only finish publication.
        self.save_release({**self.release(), "draft": True})
        self.fresh_checkout()
        tagged = self.run_ci("publish", self.revision)
        self.assertSucceeded(tagged)
        self.assertTrue(all(call["event"] not in ("POST", "DELETE", "push-tag") for call in self.calls()),
                        "tagged draft assets were deleted or uploaded")
        self.assertEqual(len(self.events("PATCH")), 1, "tagged draft must be published once")
        # The delete helper checks the tag itself, right before it deletes.
        self.save_release({**self.release(), "draft": True})
        self.reset_calls()
        guarded = run_script(os.path.join(self.project, "scripts", "release.py"),
                             ["delete-draft-assets", "42", self.revision], env=self.env, cwd=self.project)
        self.assertFailed(guarded, "already pushed")
        self.assertEqual(self.events("DELETE"), [], "delete helper deleted assets after the tag push")
        self.save_release({**self.release(), "draft": False})

    def check_late_byte_mismatch(self):
        # Known limit: a same-size byte mismatch is only caught after publication.
        # The job must still fail, and a rerun must not change the public release.
        self.empty_server()
        late = self.run_ci("publish", self.revision, FAKE_CORRUPT_DOWNLOAD="true")
        self.assertFailed(late, "does not match the locally checked output")
        self.assertFalse(self.release()["draft"], "same-size mismatch is expected to be found only after publication")
        self.fresh_checkout()
        self.assertFailed(self.run_ci("publish", self.revision, FAKE_CORRUPT_DOWNLOAD="true"))
        self.assertReadOnly("rerun modified a public release with mismatched bytes")


class CommandLineTests(TempTestCase):
    def test_usage(self):
        for arguments in ([], ["unknown"], ["build"], ["candidate", "a", "b"]):
            result = run_script("forgejo_ci.py", arguments)
            self.assertEqual(result.returncode, 1, result.stderr)
            self.assertIn("error: ", result.stderr)
            self.assertNotIn("Traceback", result.stderr)
        self.assertIn("usage: scripts/forgejo_ci.py candidate", run_script("forgejo_ci.py", []).stderr)
