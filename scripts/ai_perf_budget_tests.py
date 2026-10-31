"""Fork-only budget scope and strict measured-time boundaries."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().with_name("ai-perf-budget.py")
spec = importlib.util.spec_from_file_location("ai_perf_budget", SCRIPT)
policy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(policy)
SAGET = {"GITHUB_ACTIONS": "true", "GITHUB_REPOSITORY": "bryanroscoe/yama-phase",
         "RUNNER_ENVIRONMENT": "self-hosted", "RUNNER_NAME": "saget-yama-phase-ci"}


class BudgetTests(unittest.TestCase):
    def test_only_exact_private_saget_identity_gets_longer_budget(self):
        self.assertEqual(policy.budget_seconds(SAGET), 3600)
        for key in SAGET:
            with self.subTest(missing=key):
                env = dict(SAGET)
                del env[key]
                self.assertEqual(policy.budget_seconds(env), 1500)
        for key, value in (("GITHUB_REPOSITORY", "phase-rs/phase"),
                           ("RUNNER_ENVIRONMENT", "github-hosted"),
                           ("RUNNER_NAME", "GitHub Actions 1")):
            with self.subTest(key=key, value=value):
                self.assertEqual(policy.budget_seconds({**SAGET, key: value}), 1500)

    def run_check(self, build, run, env):
        return subprocess.run([sys.executable, str(SCRIPT), "--build-seconds", str(build),
                               "--max-run-seconds", str(run)], env=env,
                              capture_output=True, text=True)

    def test_strict_budget_boundaries(self):
        for env, limit in (({}, 1500), (SAGET, 3600)):
            for build, expected in ((limit - 6, 0), (limit - 5, 1), (limit - 4, 1)):
                with self.subTest(limit=limit, build=build):
                    result = self.run_check(build, 2, env)
                    self.assertEqual(result.returncode, expected, result.stderr)
                    self.assertEqual(json.loads(result.stdout)["budget_seconds"], limit)

    def test_cold_guard_preserves_existing_output_and_rejects_symlinks(self):
        validator = SCRIPT.with_name("validate-ai-perf-reproducibility.sh").read_text()
        guard = validator.split("# SERVER-RELEASE profile")[0]
        for mode in ("populated", "symlink", "file"):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                (root / "scripts").mkdir()
                (root / "target").mkdir()
                script = root / "scripts" / "validate.sh"
                script.write_text(guard)
                target = root / "target" / "ai"
                if mode == "populated":
                    target.mkdir()
                    (target / "sentinel").write_text("preserve")
                elif mode == "symlink":
                    target.symlink_to(root / "scripts", target_is_directory=True)
                else:
                    target.write_text("preserve")
                result = subprocess.run(["bash", str(script)], capture_output=True, text=True)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertTrue(target.exists())

    def test_missing_or_invalid_measurement_fails(self):
        for build, run in ((0, 2), (-1, 2), (2, 0), (2, "NaN")):
            with self.subTest(build=build, run=run):
                self.assertEqual(self.run_check(build, run, SAGET).returncode, 2)


if __name__ == "__main__":
    unittest.main()
