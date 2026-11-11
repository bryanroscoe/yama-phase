# Working on Yama Phase

This private repository is upstream [phase-rs/phase](https://github.com/phase-rs/phase) `main`
plus a short stack of Yama commits on top. Yama builds the exact commit recorded in its
`deploy/phase-repo` and `deploy/phase-rev`.

- Private repository: <https://github.com/bryanroscoe/yama-phase>
- Upstream: <https://github.com/phase-rs/phase>
- History before the 2026-10-28 rebuild: tag `archive/pre-sync-20261028` (the old `main`,
  based on v0.113.0). Yama releases built before the rebuild pin commits under that tag.

Both upstream licenses and authorship are retained.

## Attaching Yama's board

Yama's `GameStarted` envelope carries an optional top-level `gameplay_preferences`
extension. It initializes the client from the existing match's standing settings
without dispatching preference actions. A temporary priority hold stays separate
from standing Full Control; null marks a read-only attachment. Every accepted
reconnect replaces this preference envelope, including on a reused adapter.
Queued preference actions and late completions from the prior envelope are
discarded; later explicit changes use the new envelope. Ordinary Phase
servers omit the extension and retain device-default initialization. This is a
Yama envelope extension, not a change to the Phase wire payload or protocol version;
Yama pins the matching client and server together.

The envelope also carries the engine's per-seat `un_card_actions_enabled`
opt-in. An older host that omits it has un-card actions disabled. Reconnect
hydrates this setting without sending an engine action; a subsequent explicit
toggle still uses upstream's actor-scoped `UnAction::SetEnabled` path.

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

The fork was republished as public on 31 October 2026. Its CI runs on GitHub-hosted
runners; never route Phase CI to saget. Keep upstream release and deployment workflows
disabled unless Bryan explicitly requests them.

## Hosted card illustrations

Yama builds with `VITE_HOSTED_CARD_ART=1`. Cropped card art is shown with the artist
for the actual image's printing and face. Generated Scryfall image metadata carries
an optional `artist` per face; Yama's runtime feed supplies the same field. Missing
metadata is resolved through the shared, rate-limited Scryfall printing-ID lookup,
never by choosing the Oracle card's default printing. A missing credit leaves an
identifiable text tile. External account avatars keep their separate identity.

The hosted stylesheet preserves card pixels through their ancestor containers:
no opacity, filters, masks or blending are applied to the illustration or its
ancestors. Separate readability washes remain. Classic position/scale motion
remains; deaths use the existing text effect, and image-altering WebGL flights
and meld texture morphs are disabled in this build mode. Compact designation
badges keep their glyphs, and report thumbnails show whole cards. Ordinary Phase builds
do not enable this hosted policy. Missing back-face crops fall back to that face's
full image, keeping the illustration and its credit together. Illustrations and mana symbols load from
Scryfall at runtime; no card images are added to the bundle.
