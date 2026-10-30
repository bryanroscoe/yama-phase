#!/usr/bin/env python3
"""Exercise exact-base generation and private cache identity without compiling Rust."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import textwrap
import unittest
from unittest.mock import patch

import private_parse_baseline as baseline


class BaselineTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="phase-baseline-test-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.scratch = self.root / "scratch"
        self.scratch.mkdir()
        self.env = patch.dict(os.environ, {
            "PATH": str(self.bin) + os.pathsep + os.environ["PATH"],
            "RUNNER_TEMP": str(self.scratch),
        })
        self.env.start()
        self.addCleanup(self.env.stop)
        self.tool("rustc", "print('rustc fixture compiler A')")
        self.tool("rustup", "print('fixture-toolchain (directory override)')")
        self.tool("cargo", """
import json, os, pathlib, sys
if os.environ.get('BASELINE_TEST_FAIL') == 'true':
    sys.exit(7)
assert pathlib.Path(os.environ['CARGO_TARGET_DIR']).is_absolute()
assert os.environ['RUSTUP_TOOLCHAIN'] == 'fixture-toolchain'
if 'oracle-gen' in sys.argv:
    source = pathlib.Path('crates/engine/src/fixture.rs').read_text().strip()
    atomic = pathlib.Path('data/mtgjson/AtomicCards.json').read_text().strip()
    print(json.dumps({'source': source, 'input': atomic}))
else:
    data = json.loads(pathlib.Path('data/card-data.json').read_text())
    print(json.dumps({'cards': [data]}))
""")
        self.git("init", "-q")
        self.git("config", "user.name", "Fixture")
        self.git("config", "user.email", "fixture@example.invalid")
        self.git("config", "commit.gpgsign", "false")
        self.source = self.repo / "crates/engine/src/fixture.rs"
        self.source.parent.mkdir(parents=True)
        self.source.write_text("BASE")
        self.git("add", ".")
        self.git("commit", "-qm", "base")
        self.base = self.git("rev-parse", "HEAD")
        self.source.write_text("HEAD")
        self.git("add", ".")
        self.git("commit", "-qm", "head")
        self.atomic = self.root / "AtomicCards.json"
        self.atomic.write_text("same MTGJSON input")
        self.output = self.repo / ".baseline/coverage-data.json"

    def git(self, *args):
        return subprocess.check_output(["git", *args], cwd=self.repo, text=True).strip()

    def tool(self, name, body):
        path = self.bin / name
        path.write_text("#!/usr/bin/env python3\n" + body + "\n")
        path.chmod(0o755)

    def expected(self, ref=None):
        return baseline.identity(self.repo, ref or self.base, self.atomic)

    def build(self):
        expected = self.expected()
        baseline.build(self.repo, self.atomic, self.output, expected)
        return expected

    def test_cold_miss_builds_actual_base_with_same_input_without_touching_head(self):
        before = self.git("status", "--porcelain")
        head = self.git("rev-parse", "HEAD")
        expected = self.build()
        result = json.loads(self.output.read_text())
        self.assertEqual(result["cards"], [{"source": "BASE", "input": "same MTGJSON input"}])
        self.assertEqual(result["private_ci_baseline"], expected)
        self.assertEqual(self.source.read_text(), "HEAD")
        self.assertEqual(self.git("rev-parse", "HEAD"), head)
        self.assertEqual(list(self.scratch.iterdir()), [])
        self.output.unlink()
        self.output.parent.rmdir()
        self.assertEqual(self.git("status", "--porcelain"), before)

    def test_cache_hit_validates_without_running_cargo(self):
        expected = self.build()
        with patch.dict(os.environ, {"BASELINE_TEST_FAIL": "true"}):
            baseline.validate(self.output, expected)

    def test_source_input_and_compiler_changes_each_reject_stale_baseline(self):
        expected = self.build()
        changed_source = self.expected("HEAD")
        with self.assertRaisesRegex(ValueError, "engine_hash"):
            baseline.validate(self.output, changed_source)
        self.atomic.write_text("different MTGJSON")
        with self.assertRaisesRegex(ValueError, "input_hash"):
            baseline.validate(self.output, self.expected())
        self.atomic.write_text("same MTGJSON input")
        self.tool("rustc", "print('rustc fixture compiler B')")
        with self.assertRaisesRegex(ValueError, "compiler_hash"):
            baseline.validate(self.output, self.expected())
        self.assertNotEqual(expected["compiler_hash"], self.expected()["compiler_hash"])

    def test_equivalent_source_commit_can_share_content_addressed_baseline(self):
        expected = self.build()
        same = dict(expected, base_sha="f" * 40)
        baseline.validate(self.output, same)

    def test_base_cannot_silently_use_its_own_nightly_override(self):
        self.tool("rustc", """
import os, pathlib
is_base = pathlib.Path.cwd().name == 'private-parse-baseline-src'
if is_base and os.environ.get('RUSTUP_TOOLCHAIN') != 'fixture-toolchain':
    print('rustc fixture NIGHTLY')
else:
    print('rustc fixture compiler A')
""")
        expected = self.build()
        baseline.validate(self.output, expected)

    def test_unexpected_base_compiler_is_rejected_without_stamping_output(self):
        self.tool("rustc", """
import pathlib
print('rustc fixture NIGHTLY' if pathlib.Path.cwd().name == 'private-parse-baseline-src' else 'rustc fixture compiler A')
""")
        with self.assertRaisesRegex(ValueError, "base compiler"):
            self.build()
        self.assertFalse(self.output.exists())
        self.assertEqual(list(self.scratch.iterdir()), [])

    def test_failed_generation_preserves_previous_output_and_cleans_scratch(self):
        self.build()
        before = self.output.read_bytes()
        with patch.dict(os.environ, {"BASELINE_TEST_FAIL": "true"}):
            with self.assertRaises(subprocess.CalledProcessError):
                self.build()
        self.assertEqual(self.output.read_bytes(), before)
        self.assertEqual(list(self.scratch.iterdir()), [])

    def test_missing_or_malformed_cache_cannot_be_accepted(self):
        with self.assertRaises(FileNotFoundError):
            baseline.validate(self.output, self.expected())
        self.output.parent.mkdir()
        for value in ({}, {"cards": []}, {"cards": "not an array"}, {"cards": [{}]},
                      {"cards": [{}], "private_ci_baseline": None}):
            with self.subTest(value=value):
                self.output.write_text(json.dumps(value))
                with self.assertRaises(ValueError):
                    baseline.validate(self.output, self.expected())

    def test_interrupted_source_directory_is_preserved_and_refused(self):
        existing = self.scratch / "private-parse-baseline-src"
        existing.mkdir()
        evidence = existing / "unknown-output"
        evidence.write_text("preserve")
        with self.assertRaises(FileExistsError):
            self.build()
        self.assertEqual(evidence.read_text(), "preserve")

    def test_shipped_steps_use_shallow_merge_parents_without_origin_credentials(self):
        # A docs-only PR and a parallel main commit produce a real synthetic
        # merge. Clone it as checkout does, with only the merge and two parents.
        self.source.write_text("BASE")
        self.git("add", ".")
        self.git("commit", "-qm", "docs-only PR")
        head = self.git("rev-parse", "HEAD")
        self.git("checkout", "-q", "--detach", self.base)
        (self.repo / "main-note").write_text("parallel main change")
        self.git("add", ".")
        self.git("commit", "-qm", "main")
        main = self.git("rev-parse", "HEAD")
        self.git("merge", "--no-ff", "-qm", "synthetic PR merge", head)
        self.git("branch", "synthetic-merge")
        checkout = self.root / "shallow"
        subprocess.run(["git", "clone", "--quiet", "--depth", "2", "--branch",
                        "synthetic-merge", self.repo.as_uri(), str(checkout)], check=True)
        # Any later fetch fails. No credentials or private repository needed.
        subprocess.run(["git", "remote", "set-url", "origin", str(self.root / "no-origin")],
                       cwd=checkout, check=True)
        scripts = checkout / "scripts"
        scripts.mkdir()
        origin_scripts = Path(__file__).resolve().parent
        for name in ("private_parse_baseline.py", "engine-source-hash.sh", "parse-diff-base.sh"):
            shutil.copy2(origin_scripts / name, scripts / name)
        data = checkout / "data/mtgjson"
        data.mkdir(parents=True)
        shutil.copy2(self.atomic, data / "AtomicCards.json")
        output = self.root / "step-output"
        output.touch()
        env = dict(os.environ, EVENT_NAME="pull_request", PR_HEAD=head, HEAD_SHA=head,
                   PR_NUMBER="8", PRIVATE_CI="true", GITHUB_OUTPUT=str(output))
        workflow = (origin_scripts.parent / ".github/workflows/ci.yml").read_text()
        for name in ("Identify private parse baseline", "Parse-detail diff vs base baseline"):
            step = workflow.split("      - name: " + name + "\n", 1)[1].split("\n      - ", 1)[0]
            shell = textwrap.dedent(step.split("        run: |\n", 1)[1])
            subprocess.run(["bash", "-c", shell], cwd=checkout, env=env,
                           stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, check=True)
        self.assertIn("base_sha=" + main, output.read_text())
        self.assertIn("cache_key=private-parse-baseline-v1-", output.read_text())
        self.assertIn("produced=false", output.read_text())


if __name__ == "__main__":
    unittest.main()
