#!/usr/bin/env python3
"""Check the measured cold-build + gate budget without changing perf thresholds."""
import argparse
import json
import os


def budget_seconds(env):
    # The longer budget is a fork-only host decision, not a portable override.
    private_saget = (
        env.get("GITHUB_ACTIONS") == "true"
        and env.get("GITHUB_REPOSITORY") == "bryanroscoe/yama-phase"
        and env.get("RUNNER_ENVIRONMENT") == "self-hosted"
        and env.get("RUNNER_NAME") == "saget-yama-phase-ci"
    )
    return 3600 if private_saget else 1500


def positive_seconds(value):
    seconds = int(value)
    if seconds <= 0:
        raise argparse.ArgumentTypeError("measured seconds must be positive")
    return seconds


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--build-seconds", type=positive_seconds, required=True)
    parser.add_argument("--max-run-seconds", type=positive_seconds, required=True)
    args = parser.parse_args()
    limit = budget_seconds(os.environ)
    measured = args.build_seconds + 2.5 * args.max_run_seconds
    passed = measured < limit
    print(json.dumps(dict(build_seconds=args.build_seconds,
                          max_run_seconds=args.max_run_seconds,
                          budget_seconds=limit, measured_budget_seconds=measured,
                          budget_pass=passed), indent=2))
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
