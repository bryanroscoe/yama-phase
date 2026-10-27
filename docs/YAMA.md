# Working on Yama Phase

This private repository retains the upstream Phase history and Yama's engine changes.
It replaces the patch series formerly maintained inside `bryanroscoe/yama`.

- Private repository: <https://github.com/bryanroscoe/yama-phase>
- Upstream project: <https://github.com/phase-rs/phase>
- Initial Yama source: `9b2073b0986fe580d495fce2fa58b5762e24e3eb`
- Integration: Yama records this repository and an exact commit in `deploy/phase-repo` and
  `deploy/phase-rev`; development checkouts normally place `yama` and `yama-phase` side by side.

Both upstream licenses and authorship are retained. The rest of the upstream README and
contributor documentation remain available for the engine and client architecture.

## Contribute here

```sh
git clone https://github.com/bryanroscoe/yama-phase.git
cd yama-phase
git remote add upstream https://github.com/phase-rs/phase.git
git switch -c perf/describe-the-change
```

Make a focused change, run the relevant local checks from CONTRIBUTING, and open a PR against
`bryanroscoe/yama-phase:main`. Shared AI plumbing and engine changes use the fresh-context
implementation/review workflow described in AGENTS.md. Keep the benchmark fixtures and tests
with the change, so another contributor can reproduce the result.

Preserve the existing card-data and game-state contracts. Card definitions, rules documents,
private saved games and credentials remain local ignored inputs. Do not add them to a commit.
The ordinary code tests can use the repository's existing fixtures; full game checks require
the same pinned card export used by Yama.

## Performance work

The goal is at least 2–3× Forge's throughput on the relevant Commander workload. Rust alone
does not establish that result. Record machine, engine and card-data revisions, AI settings,
game seeds, concurrency, complete job time, CPU time, memory and every failure or timeout.
Include slow games rather than comparing only the successful fast subset.

For a preserving optimization, compare Phase before and after on identical seeds and lists:
actions, outcomes and RNG behavior must agree. Compare Phase and Forge as separate programs
under a matched resource/concurrency budget; their AIs can choose different actions. A healthy
worker, a parser coverage percentage or two runs repeating an error is not gameplay acceptance.

The initial bottleneck investigation found repeated legal-candidate enumeration and expensive
mana-payment simulation. The live game `449cfd5a-23-t`, seed `3160109258497874`, took 432.265 s
on the original deployed build. It finished with winner 1, turn 44 and 1,903 actions. Yama's
full-game and saved-regression tests are the consumer-side acceptance checks.

## Keep upstream contributions practical

Fetch `upstream` without rewriting shared history. Review an upstream update on a separate
branch, run the engine and Yama checks, then merge it into this fork deliberately. Keep generic
engine/AI changes separate from fork-only documentation and Yama integration changes.

When Bryan chooses to contribute upstream, a focused public contribution branch can carry
the generic commits. Do not publish private fixtures, configuration or fork-only material.
This private repository is independent of GitHub's public fork network.

## Automation

The upstream workflows include scheduled jobs and production publishing. GitHub Actions is
disabled on this repository. Local checks remain the verification path until fork-specific
automation is deliberately configured. Do not enable upstream deployment workflows here.
