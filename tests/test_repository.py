"""Repository contract tests: workflows, Dependabot, justfile, package and Markdown links.

Run: python3 -m unittest discover --start-directory tests --pattern 'test_*.py'
Needs PyYAML. `just check-full` runs these tests; `just check` does not.
"""

from __future__ import annotations

import glob
import json
import os
import re
import tomllib
import unittest

import yaml

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SHA_ACTION = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_./-]+@[0-9a-f]{40}\Z")
LINK = re.compile(r"\]\((?P<target>[^\s)]+)\)")
URL_SCHEME = re.compile(r"[A-Za-z][A-Za-z0-9+.-]*:")
READ_ONLY = {"contents": "read"}


class WorkflowLoader(yaml.SafeLoader):
    """A YAML 1.2 style safe loader.

    PyYAML follows YAML 1.1: `on`, `off`, `yes` and `no` become booleans, and
    dates become datetime objects. Workflow runners read `on:` as a string, so
    this loader keeps those values as strings. Only true/false are booleans.
    """


WorkflowLoader.yaml_implicit_resolvers = {
    first: [(tag, regexp) for tag, regexp in resolvers
            if tag not in ("tag:yaml.org,2002:bool", "tag:yaml.org,2002:timestamp")]
    for first, resolvers in yaml.SafeLoader.yaml_implicit_resolvers.items()
}
WorkflowLoader.add_implicit_resolver("tag:yaml.org,2002:bool", re.compile(r"^(?:true|True|TRUE|false|False|FALSE)$"),
                                     list("tTfF"))


def read_text(path: str) -> str:
    with open(os.path.join(ROOT, path), encoding="utf-8") as file:
        return file.read()


def load_yaml(path: str):
    return yaml.load(read_text(path), Loader=WorkflowLoader)


def to_json(value) -> str:
    return json.dumps(value, sort_keys=True)


class LoaderTests(unittest.TestCase):
    def test_yaml_1_1_values_stay_strings(self):
        document = yaml.load("on: {push: null}\nyes: no\noff: 2026-01-01\nbool: false\n", Loader=WorkflowLoader)
        self.assertEqual(document, {"on": {"push": None}, "yes": "no", "off": "2026-01-01", "bool": False})

    def test_workflow_keys_are_strings(self):
        for path in (".github/workflows/ci.yml", ".forgejo/workflows/ci.yaml",
                     ".forgejo/workflows/github-mirror.yaml", ".github/dependabot.yml"):
            with self.subTest(path=path):
                document = load_yaml(path)
                self.assertNotIn(True, document, "workflow keys must parse as strings")
                if "workflows" in path:
                    self.assertIn("on", document, "workflow must define its events")


class PackageContractTests(unittest.TestCase):
    def setUp(self):
        with open(os.path.join(ROOT, "Cargo.toml"), "rb") as file:
            self.manifest = tomllib.load(file)
        self.package = self.manifest["package"]
        self.dependencies = self.manifest["dependencies"]

    def test_publication_is_disabled(self):
        self.assertIs(self.package["publish"], False, "package publication must remain disabled")

    def test_no_library_api(self):
        self.assertFalse(os.path.exists(os.path.join(ROOT, "src/lib.rs")),
                         "fjx must remain a single binary without a library API")

    def test_license(self):
        self.assertEqual(self.package["license"], "MIT", "license metadata must match LICENSE")
        self.assertTrue(read_text("LICENSE").startswith("MIT License"), "MIT license text is missing")

    def test_readme_exists(self):
        self.assertTrue(os.path.exists(os.path.join(ROOT, self.package["readme"])), "package readme does not exist")

    def test_dependency_allowlist(self):
        self.assertEqual(sorted(self.dependencies), ["lexopt", "rpassword", "serde", "serde_json", "ureq"],
                         "runtime dependency allowlist changed")

    def test_ureq_uses_rustls_only(self):
        ureq = self.dependencies["ureq"]
        self.assertIs(ureq["default-features"], False, "ureq default features must stay disabled")
        self.assertEqual(ureq["features"], ["tls"], "ureq must use the Rustls TLS feature")

    def test_dependencies_are_pinned(self):
        for name, dependency in self.dependencies.items():
            with self.subTest(dependency=name):
                requirement = dependency if isinstance(dependency, str) else dependency["version"]
                self.assertTrue(requirement.startswith("="), "runtime dependency versions must be pinned exactly")


class GitHubCiContractTests(unittest.TestCase):
    path = ".github/workflows/ci.yml"

    def setUp(self):
        self.workflow = load_yaml(self.path)
        self.events = self.workflow["on"]

    def test_triggers(self):
        self.assertIn("pull_request", self.events, "pull requests must run CI")
        self.assertIn("push", self.events, "pushes must run CI")
        self.assertIn("main", self.events["push"]["branches"], "main must run CI")
        self.assertNotIn("pull_request_target", self.events,
                         "do not run untrusted changes in a privileged PR workflow")
        self.assertNotIn("workflow_run", self.events, "CI must not use a privileged follow-up trigger")

    def test_read_only_permissions(self):
        self.assertEqual(self.workflow["permissions"], READ_ONLY, "CI must have read-only contents permissions")

    def test_no_secrets(self):
        self.assertNotIn("secrets.", read_text(self.path), "CI must not consume repository secrets")

    def test_job_timeouts(self):
        for name, job in self.workflow["jobs"].items():
            with self.subTest(job=name):
                timeout = job.get("timeout-minutes")
                self.assertTrue(type(timeout) is int and 0 < timeout <= 30, "jobs need a bounded timeout")

    def test_job_permissions(self):
        for name, job in self.workflow["jobs"].items():
            with self.subTest(job=name):
                permissions = job.get("permissions")
                self.assertEqual(READ_ONLY if permissions is None else permissions, READ_ONLY,
                                 "job permissions must not elevate privileges")

    def test_actions_are_pinned(self):
        for name, job in self.workflow["jobs"].items():
            for step in job["steps"]:
                action = step.get("uses") or ""
                if action:
                    with self.subTest(job=name, action=action):
                        self.assertRegex(action, SHA_ACTION, "actions must be pinned to a full commit SHA")

    def test_checkout_drops_credentials(self):
        for name, job in self.workflow["jobs"].items():
            for step in job["steps"]:
                if (step.get("uses") or "").startswith("actions/checkout@"):
                    with self.subTest(job=name):
                        self.assertIs((step.get("with") or {}).get("persist-credentials"), False,
                                      "checkout must not leave credentials for PR code")

    def test_runs_just_check(self):
        steps = [step for job in self.workflow["jobs"].values() for step in job["steps"]
                 if (step.get("run") or "").strip() == "just check"]
        self.assertGreater(len(steps), 0, "CI must run the same check command as contributors")

    def test_justfile_recipes(self):
        justfile = read_text("justfile")
        self.assertIn("\ncheck:\n    bash scripts/check.sh\n", justfile, "just check must run the check script")
        self.assertIn("\ncheck-full:\n    bash scripts/check.sh --full\n", justfile,
                      "just check-full must run the full check script")

    def test_dependabot(self):
        updates = load_yaml(".github/dependabot.yml")["updates"]
        self.assertEqual(sorted(update["package-ecosystem"] for update in updates), ["cargo", "github-actions"],
                         "monitor Cargo and Actions dependencies")
        for update in updates:
            with self.subTest(ecosystem=update["package-ecosystem"]):
                self.assertEqual(update["directory"], "/", "Dependabot must target the standalone repository root")


class ForgejoCiContractTests(unittest.TestCase):
    path = ".forgejo/workflows/ci.yaml"

    def setUp(self):
        self.workflow = load_yaml(self.path)
        self.events = self.workflow["on"]
        self.jobs = self.workflow["jobs"]
        self.checks = self.jobs["checks"]
        self.release = self.jobs["release"]
        self.mirror = self.jobs["github-mirror"]

    def test_triggers(self):
        self.assertTrue("pull_request" in self.events and "push" in self.events, "Forgejo must check PRs and main")
        self.assertNotIn("pull_request_target", self.events, "Forgejo must not use privileged PR execution")
        self.assertEqual(self.events["push"]["branches"], ["main"], "Forgejo push checks must target main")

    def test_checks_job(self):
        self.assertEqual(self.checks["timeout-minutes"], 5, "PR checks must have a five-minute budget")
        self.assertEqual(self.checks["runs-on"], "fjx-ci", "checks must use the prepared runner")
        self.assertTrue(any((step.get("run") or "") == "just check-full" for step in self.checks["steps"]),
                        "Forgejo must run the CLI and release-tool checks")

    def test_release_job_gates(self):
        self.assertEqual(self.release["needs"], "checks", "release publication must wait for checks")
        self.assertNotEqual(self.release["runs-on"], self.checks["runs-on"],
                            "release builds must have a separate runner slot")
        for condition in ("forgejo.event_name == 'push'", "forgejo.ref == 'refs/heads/main'",
                          "needs.checks.outputs.release == 'true'"):
            with self.subTest(condition=condition):
                self.assertIn(condition, self.release["if"], "release must require a checked main version bump")

    def test_candidate_and_publisher(self):
        candidate = next((step for step in self.checks["steps"] if (step.get("id") or "") == "version"), None)
        self.assertIsNotNone(candidate, "version check must run the tested candidate step")
        publisher = self.release["steps"][-1]
        self.assertTrue(candidate["run"].startswith("python3 scripts/forgejo_ci.py candidate "),
                        "version check must run the tested candidate step")
        self.assertTrue(publisher["run"].startswith("python3 scripts/forgejo_ci.py publish "),
                        "release job must run the tested publisher")
        for name, step in (("candidate", candidate), ("publisher", publisher)):
            with self.subTest(step=name):
                env = step["env"]
                self.assertTrue(env["BEFORE_SHA"] == "${{ forgejo.event.before }}"
                                and env["CHECKOUT_SHA"] == "${{ forgejo.sha }}",
                                "candidate and publisher must receive the same exact push range")
                self.assertTrue(env["FORGEJO_EVENT_NAME"] == "${{ forgejo.event_name }}"
                                and env["FORGEJO_REF"] == "${{ forgejo.ref }}",
                                "publication context guards must receive the actual workflow event")

    def test_main_is_not_canceled(self):
        self.assertEqual(self.workflow["concurrency"]["cancel-in-progress"], "${{ forgejo.ref != 'refs/heads/main' }}",
                         "main publication must not be canceled by later pushes")

    # Forgejo starts no workflow for release events made by the job token, so
    # the release run copies the release to GitHub itself, once it is final.
    def test_github_mirror_job(self):
        self.assertEqual(self.mirror["needs"], "release", "GitHub copy must wait for the final Forgejo release")
        self.assertIn(self.mirror.get("if"), (None, ""), "GitHub copy must run only when the release job succeeds")
        self.assertTrue(self.mirror["runs-on"] == "fjx-release" and self.mirror["timeout-minutes"] == 20,
                        "GitHub copy must use the bounded release runner")
        self.assertNotIn("secrets.", to_json(self.mirror["steps"][0]), "checkout must not consume the GitHub credential")
        copier = self.mirror["steps"][-1]
        self.assertEqual(copier["env"]["GH_RELEASE_MIRROR_TOKEN"], "${{ secrets.GH_RELEASE_MIRROR_TOKEN }}",
                         "GitHub credential must be scoped to the copy step")
        self.assertEqual(copier["run"], "python3 scripts/github_mirror.py", "release run must use the tested copier")

    def test_other_jobs_have_no_secrets(self):
        other_jobs = {name: job for name, job in self.jobs.items() if name != "github-mirror"}
        self.assertNotIn("secrets.", to_json(other_jobs),
                         "build, test and publication jobs must not require repository secrets")

    def test_no_apt_get(self):
        self.assertNotIn("apt-get", read_text(self.path), "tool installation must stay outside PR jobs")


class GitHubMirrorContractTests(unittest.TestCase):
    path = ".forgejo/workflows/github-mirror.yaml"

    def setUp(self):
        self.workflow = load_yaml(self.path)
        self.events = self.workflow["on"]
        self.job = self.workflow["jobs"]["mirror"]

    def test_triggers(self):
        self.assertEqual(sorted(self.events), ["push", "release", "schedule", "workflow_dispatch"],
                         "release copying must have recovery and must never run PR code")
        self.assertEqual(self.events["push"]["branches"], ["main"], "only trusted main pushes may trigger the mirror")
        self.assertEqual(self.events["release"]["types"], ["published", "edited"],
                         "mirror public releases and metadata edits")
        self.assertEqual(self.events["schedule"][0]["cron"], "23 */6 * * *",
                         "missed releases need six-hour reconciliation")

    def test_not_canceled(self):
        self.assertIs(self.workflow["concurrency"]["cancel-in-progress"], False, "copying must not be canceled midway")

    def test_runner(self):
        self.assertTrue(self.job["runs-on"] == "fjx-release" and self.job["timeout-minutes"] == 20,
                        "mirror must use the bounded existing release runner")

    def test_checkout(self):
        checkout = self.job["steps"][0]
        self.assertIn("git fetch --no-tags origin refs/heads/main", checkout["run"],
                      "release events must never execute code from old release tags")
        self.assertNotIn("secrets.", to_json(checkout), "checkout must not consume the GitHub credential")

    def test_copier(self):
        copier = self.job["steps"][1]
        self.assertEqual(copier["env"]["GH_RELEASE_MIRROR_TOKEN"], "${{ secrets.GH_RELEASE_MIRROR_TOKEN }}",
                         "GitHub credential must be scoped to the copy step")
        self.assertEqual(copier["run"], "python3 scripts/github_mirror.py", "workflow must run the tested copier")


class DocumentationLinkTests(unittest.TestCase):
    def test_local_links_exist(self):
        files = sorted(glob.glob("*.md", root_dir=ROOT) + glob.glob("docs/**/*.md", root_dir=ROOT, recursive=True)
                       + glob.glob(".github/*.md", root_dir=ROOT))
        self.assertIn("README.md", files)
        for file in files:
            # Check relative inline Markdown file links, not external URLs or heading anchors.
            for match in LINK.finditer(read_text(file)):
                target = match["target"]
                if URL_SCHEME.match(target) or target.startswith("#"):
                    continue
                relative = target.split("#")[0]
                with self.subTest(file=file, link=target):
                    self.assertTrue(os.path.exists(os.path.join(ROOT, os.path.dirname(file), relative)),
                                    f"broken file link in {file}: {target}")


if __name__ == "__main__":
    unittest.main()
