#!/usr/bin/env python3
"""Keep parse baselines private and rebuild an exact base when its cache expires."""

import argparse
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile


def compiler_fingerprint(root, env=None):
    env = os.environ if env is None else env
    compiler = subprocess.check_output(["rustc", "-vV"], cwd=root, env=env)
    settings = json.dumps({
        "tool_codegen_units": env.get("CARGO_PROFILE_TOOL_CODEGEN_UNITS"),
    }, sort_keys=True).encode()
    return hashlib.sha256(compiler + b"\n" + settings).hexdigest()


def identity(root, ref, atomic):
    sha = subprocess.check_output(
        ["git", "rev-parse", "--verify", ref + "^{commit}"], cwd=root, text=True
    ).strip()
    if not re.fullmatch(r"[0-9a-f]{40}", sha):
        raise ValueError("baseline ref is not a commit")
    engine_hash = subprocess.check_output(
        ["bash", str(Path(__file__).with_name("engine-source-hash.sh")), sha],
        cwd=root, text=True,
    ).strip()
    digest = hashlib.sha256()
    with atomic.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return {
        "base_sha": sha,
        "engine_hash": engine_hash,
        "input_hash": digest.hexdigest(),
        "compiler_hash": compiler_fingerprint(root),
    }


def read_coverage(path):
    with path.open() as stream:
        data = json.load(stream)
    if not isinstance(data, dict) or not isinstance(data.get("cards"), list) or not data["cards"]:
        raise ValueError("baseline must contain a nonempty coverage cards array")
    return data


def store(coverage, output, expected):
    data = read_coverage(coverage)
    data["private_ci_baseline"] = expected
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(mode="w", dir=output.parent, delete=False) as stream:
            temporary = Path(stream.name)
            json.dump(data, stream, separators=(",", ":"))
            stream.write("\n")
        os.replace(temporary, output)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def validate(path, expected):
    data = read_coverage(path)
    recorded = data.get("private_ci_baseline", {})
    if not isinstance(recorded, dict):
        raise ValueError("private baseline metadata is not an object")
    # Different main commits with identical engine/input/compiler bytes share a
    # baseline. Never reuse one after a parser, MTGJSON, or toolchain change.
    for key in ("engine_hash", "input_hash", "compiler_hash"):
        if recorded.get(key) != expected[key]:
            raise ValueError("private baseline identity mismatch: " + key)
    if not re.fullmatch(r"[0-9a-f]{40}", recorded.get("base_sha", "")):
        raise ValueError("private baseline has no source commit")


@contextmanager
def source_directory():
    # A stable source path keeps Cargo's package identity stable across cold
    # baselines. A random manifest path would leave another set of workspace
    # artifacts in the shared target on every miss. Each runner executes one
    # job at a time; never replace unexpected leftovers after an interruption.
    base = Path(os.environ["RUNNER_TEMP"]) / "private-parse-baseline-src"
    base.mkdir(mode=0o700)
    try:
        yield base
    finally:
        shutil.rmtree(base)


def build(root, atomic, output, expected):
    # This job has already built the head's diff binary. Compile the base
    # sequentially into the SAME target cache; no concurrent divergent builds
    # or duplicate dependency tree. The base never writes into the head tree.
    env = os.environ.copy()
    env["CARGO_TARGET_DIR"] = str((root / "target").resolve())
    # setup-rust-toolchain uses a directory override in the HEAD checkout.
    # The archived base has its own rust-toolchain.toml outside that override.
    env["RUSTUP_TOOLCHAIN"] = subprocess.check_output(
        ["rustup", "show", "active-toolchain"], cwd=root, text=True
    ).split()[0]
    with source_directory() as base:
        with subprocess.Popen(["git", "archive", expected["base_sha"]], cwd=root,
                              stdout=subprocess.PIPE) as archive:
            try:
                subprocess.run(["tar", "-x", "-C", str(base)], stdin=archive.stdout, check=True)
            finally:
                archive.stdout.close()
            if archive.wait() != 0:
                raise RuntimeError("cannot export baseline commit")
        if compiler_fingerprint(base, env) != expected["compiler_hash"]:
            raise ValueError("base compiler differs from the head compiler")
        data = base / "data"
        (data / "mtgjson").mkdir(parents=True, exist_ok=True)
        (data / "mtgjson/AtomicCards.json").symlink_to(atomic.resolve())
        cargo = ["cargo", "run", "--profile", "tool", "--features", "cli", "--bin"]
        with (data / "card-data.json").open("w") as stream:
            subprocess.run(cargo + ["oracle-gen", "--", "data/", "--stats", "--names-out",
                                    "data/card-names.json"], cwd=base, env=env,
                           stdout=stream, check=True)
        coverage = data / "coverage-data.json"
        with coverage.open("w") as stream:
            subprocess.run(cargo + ["coverage-report", "--", "data/", "--all"],
                           cwd=base, env=env, stdout=stream, check=True)
        store(coverage, output, expected)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("identify", "validate", "store", "build"))
    parser.add_argument("--ref", required=True)
    parser.add_argument("--input", type=Path, default=Path("data/mtgjson/AtomicCards.json"))
    parser.add_argument("--output", type=Path, default=Path(".private-parse-baseline/coverage-data.json"))
    parser.add_argument("--coverage", type=Path, default=Path("data/coverage-data.json"))
    args = parser.parse_args()
    root = Path(subprocess.check_output(["git", "rev-parse", "--show-toplevel"], text=True).strip())
    expected = identity(root, args.ref, args.input)
    if args.command == "identify":
        for key, value in expected.items():
            print(key + "=" + value)
        print("cache_key=private-parse-baseline-v1-" + "-".join(
            expected[key] for key in ("engine_hash", "input_hash", "compiler_hash")))
    elif args.command == "validate":
        try:
            validate(args.output, expected)
        except (OSError, ValueError) as error:
            print("Private baseline unavailable: " + str(error), file=sys.stderr)
            return 1
    elif args.command == "store":
        store(args.coverage, args.output, expected)
    else:
        build(root, args.input, args.output, expected)
    return 0


if __name__ == "__main__":
    sys.exit(main())
