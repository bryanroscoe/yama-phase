"""Verify shared WASM compilation, binding paths and failure propagation without Rust."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


FAKE_TOOL = '''#!/usr/bin/env python3
import json, os, pathlib, shutil, sys
tool = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
with open(os.environ["BUILD_CALLS"], "a") as f:
    f.write(json.dumps({"tool": tool, "args": args}) + "\\n")
if tool == "cargo":
    if os.environ.get("FAIL_BUILD"):
        sys.exit(42)
    profile = "release" if "--release" in args else args[args.index("--profile") + 1]
    out = pathlib.Path(os.environ["CARGO_TARGET_DIR"]) / "wasm32-unknown-unknown" / profile
    out.mkdir(parents=True, exist_ok=True)
    pages = int(os.environ.get("MEMORY_PAGES", "300"))
    encoded = bytearray()
    while True:
        part = pages & 127
        pages >>= 7
        encoded.append(part | (128 if pages else 0))
        if not pages: break
    wasm = b"\\0asm\\1\\0\\0\\0" + bytes([5, 2 + len(encoded), 1, 0]) + encoded
    for i, arg in enumerate(args):
        if arg == "--package":
            (out / (args[i + 1].replace("-", "_") + ".wasm")).write_bytes(wasm)
elif tool == "wasm-bindgen":
    out = pathlib.Path(args[args.index("--out-dir") + 1])
    name = args[args.index("--out-name") + 1]
    out.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(args[-1], out / (name + "_bg.wasm"))
    (out / (name + ".js")).write_text("// fixture")
'''


class WasmBuildTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="phase-wasm-build-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.script = self.root / "build-wasm.sh"
        shutil.copyfile(Path(__file__).resolve().parents[1] / "build-wasm.sh", self.script)
        tools = self.root / "bin"
        tools.mkdir()
        for name in ("cargo", "wasm-bindgen", "wasm-opt"):
            tool = tools / name
            tool.write_text(FAKE_TOOL)
            tool.chmod(0o755)
        self.calls = self.root / "calls.jsonl"
        self.env = {**os.environ, "PATH": str(tools) + os.pathsep + os.environ["PATH"],
                    "CARGO_BUILD_JOBS": "1", "CARGO_TARGET_DIR": str(self.root / "cache with spaces"),
                    "BUILD_CALLS": str(self.calls)}

    def run_build(self, *args):
        return subprocess.run(["bash", str(self.script), *args], cwd=self.root,
                              env=self.env, capture_output=True, text=True)

    def journal(self):
        return [json.loads(line) for line in self.calls.read_text().splitlines()]

    def check_profile(self, *args):
        result = self.run_build(*args)
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.journal()
        cargo = [c for c in calls if c["tool"] == "cargo"]
        self.assertEqual(len(cargo), 1, "two Cargo calls produce incompatible dependency feature graphs")
        command = cargo[0]["args"]
        packages = [command[i + 1] for i, word in enumerate(command) if word == "--package"]
        self.assertEqual(set(packages), {"engine-wasm", "draft-wasm"})
        self.assertEqual(len([c for c in calls if c["tool"] == "wasm-bindgen"]), 2)
        for name in ("engine_wasm", "draft_wasm"):
            self.assertTrue((self.root / "client/src/wasm" / (name + "_bg.wasm")).is_file())
        return command, calls

    def test_default_profile_builds_both_roots_once(self):
        command, calls = self.check_profile()
        self.assertEqual(command[command.index("--profile") + 1], "wasm-dev")
        self.assertFalse(any(c["tool"] == "wasm-opt" for c in calls))

    def test_release_keeps_release_and_optimization_semantics(self):
        command, calls = self.check_profile("release")
        self.assertIn("--release", command)
        self.assertEqual(len([c for c in calls if c["tool"] == "wasm-opt"]), 2)

    def test_compile_failure_prevents_binding(self):
        self.env["FAIL_BUILD"] = "1"
        self.assertEqual(self.run_build().returncode, 42)
        self.assertEqual([c["tool"] for c in self.journal()], ["cargo"])

    def test_stack_guard_still_rejects_a_small_module(self):
        self.env["MEMORY_PAGES"] = "17"
        result = self.run_build()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("stack-size rustflags", result.stderr)
        self.assertFalse((self.root / "client/src/wasm/draft_wasm.js").exists())


if __name__ == "__main__":
    unittest.main()
