"""Shared helpers for the release-tool tests (tests/test_*.py).

Importing this module puts scripts/ on sys.path, so tests can import the
script modules (release, package_release, release_smoke) directly.
"""

from __future__ import annotations

import os
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SCRIPTS = os.path.join(ROOT, "scripts")
FIXTURES = os.path.join(ROOT, "tests", "fixtures")
if SCRIPTS not in sys.path:
    sys.path.insert(0, SCRIPTS)

# Tests must never see a real credential or a fixture override from the caller.
CLEARED_VARIABLES = ("RELEASE_WORKSPACE_ROOT", "FJX_PACKAGE_BIN_ROOT", "FJX_CONTAINER_RUNTIME", "FJX_TOKEN",
                     "FORGEJO_TOKEN", "GITEA_TOKEN", "GITHUB_TOKEN", "ACTIONS_RUNTIME_TOKEN",
                     "GH_RELEASE_MIRROR_TOKEN", "FORGEJO_API_URL", "FORGEJO_REPOSITORY", "SOURCE_DATE_EPOCH")


def install_tool(bin_dir: str, name: str, module: str, function: str) -> str:
    """Writes an executable `name` that runs fixtures/<module>.py:<function>(argv)."""
    os.makedirs(bin_dir, exist_ok=True)
    path = os.path.join(bin_dir, name)
    with open(path, "w", encoding="utf-8") as file:
        file.write(f"#!{sys.executable}\nimport sys\nsys.path.insert(0, {FIXTURES!r})\n"
                   f"from {module} import {function}\nsys.exit({function}(sys.argv[1:]))\n")
    os.chmod(path, 0o755)
    return path


def base_env(overrides: dict[str, str | None] | None = None) -> dict[str, str]:
    environment = {name: value for name, value in os.environ.items() if name not in CLEARED_VARIABLES}
    for name, value in (overrides or {}).items():
        if value is None:
            environment.pop(name, None)
        else:
            environment[name] = value
    return environment


def with_path(bin_dir: str, environment: dict[str, str] | None = None) -> str:
    current = (environment or os.environ).get("PATH", "")
    return f"{bin_dir}{os.pathsep}{current}"


def run_script(script: str, arguments: list[str], env: dict[str, str | None] | None = None,
               cwd: str | None = None) -> subprocess.CompletedProcess:
    """Runs scripts/<script> (or an absolute script path) with Python; captures text output."""
    path = script if os.path.isabs(script) else os.path.join(SCRIPTS, script)
    return subprocess.run([sys.executable, path, *arguments], capture_output=True, text=True,
                          env=base_env(env), cwd=cwd)


def real_program(name: str) -> str:
    path = shutil.which(name)
    if path is None:
        raise unittest.SkipTest(f"{name} is required")
    return path


class TempTestCase(unittest.TestCase):
    """A test case with a private temporary directory, removed after each test."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="fjx-test-")
        self.addCleanup(shutil.rmtree, self.tmp, True)

    def path(self, *parts: str) -> str:
        return os.path.join(self.tmp, *parts)

    def mkdir(self, *parts: str) -> str:
        path = self.path(*parts)
        os.makedirs(path, exist_ok=True)
        return path

    def assertSucceeded(self, result: subprocess.CompletedProcess):
        self.assertEqual(result.returncode, 0, f"stdout: {result.stdout}\nstderr: {result.stderr}")

    def assertFailed(self, result: subprocess.CompletedProcess, message: str | None = None):
        self.assertNotEqual(result.returncode, 0, f"command unexpectedly passed: {result.stdout}")
        if message is not None:
            self.assertIn(message, result.stderr)
        self.assertNotIn("Traceback", result.stderr)


def write(path: str, text: str | bytes, mode: int | None = None) -> str:
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as file:
        file.write(text.encode() if isinstance(text, str) else text)
    if mode is not None:
        os.chmod(path, mode)
    return path


def read(path: str) -> bytes:
    with open(path, "rb") as file:
        return file.read()


def is_executable(path: str) -> bool:
    return bool(os.stat(path).st_mode & stat.S_IXUSR)
