# Working on Yama Phase

This private repository is upstream [phase-rs/phase](https://github.com/phase-rs/phase) `main`
plus a short stack of Yama commits on top. Yama builds the exact commit recorded in its
`deploy/phase-repo` and `deploy/phase-rev`.

- Private repository: <https://github.com/bryanroscoe/yama-phase>
- Upstream: <https://github.com/phase-rs/phase>
- History before the 2026-10-28 rebuild: tag `archive/pre-sync-20261028` (the old `main`,
  based on v0.113.0). Yama releases built before the rebuild pin commits under that tag.

Both upstream licenses and authorship are retained.

## The stack

Every commit on top of upstream is one of two kinds:

- **Upstream candidates**: one card or one class of behaviour each, written to upstream's rules
  (`CLAUDE.md`, `CONTRIBUTING.md`, `docs/AI-CONTRIBUTOR.md`), so that each can become one PR.
- **Fork-only** commits, whose subjects start with `fork:`: the stable-Rust build, these docs,
  and changes upstream would not take (sentence-rewrite parsers Yama's decks need, Yama-specific
  AI tuning).

[UPSTREAM-QUEUE.md](UPSTREAM-QUEUE.md) lists each candidate, its review status and what is left
before it can be sent.

## Keeping up with upstream

Rebase the stack on upstream; don't merge upstream into it.

```sh
git fetch upstream
git switch -c sync/upstream-$(date +%Y%m%d) main
git rebase upstream/main          # resolve conflicts commit by commit
```

- Drop a commit when upstream has fixed the same thing (say so in the sync PR description).
- Build and test the result. On Bryan's Mac, the engine's test build needs about 12 GiB, more
  than the default compiler guard. Type-check with `cargo +stable check -p phase-engine --test integration`
  and `-p phase-ai`. Run the full tests in one approved window, or on a larger host.
- Tag the current `main` as `archive/pre-sync-<date>`, push the tag, then replace `main` with
  the rebased branch (`git push --force-with-lease`). The tags keep every commit Yama has pinned
  reachable.
- Update Yama's `deploy/phase-rev` in a Yama PR, and rebuild the pinned card data there.

Use `cargo +stable` (upstream pins a nightly with cranelift; the `fork:` build commit removes
that need), one Cargo job, and low debug info.

## Sending a change upstream

Only with Bryan's go-ahead. Upstream requires every engine change to go through its
`/engine-implementer` pipeline: fresh-context plan, plan review, implementation and implementation
review. It also needs a final `/review-engine-impl` pass on the PR head, Gate A
(`./scripts/check-parser-combinators.sh`), a PR body from `.github/PULL_REQUEST_TEMPLATE.md`
with accurate `Model:`/`Tier:`/`Thinking:` lines, and one card or class per PR. Our commit is the
reference for that run, not a substitute for it. Cut the branch from `upstream/main` in a public fork, and never
push private fixtures, Yama configuration or `fork:` commits there.

Many of these fixes were found by differential testing against
[Forge](https://github.com/Card-Forge/forge): Yama's referee runs the same board and moves in
both engines and compares the results. That work is clean-room. Forge is GPL and phase is MIT, so
no Forge code or card script may reach this repository, even rewritten. Whoever reads Forge's
code doesn't write phase fixes. Fixes come from the differences, the Oracle text and the
Comprehensive Rules.

## Automation

Bryan requested private-project CI on saget on 2026-10-29. Only core CI, the game-AI gates
and trusted action-pin audit are enabled for this fork; all other inherited workflows stay
disabled. The inherited workflows target upstream infrastructure. Do not enable upstream's
release or deployment workflows. See [private CI](private-ci.md) for the backend switch,
stable Rust toolchain and isolation boundaries.
