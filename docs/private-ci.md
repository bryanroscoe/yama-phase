# Private fork CI

The `bryanroscoe/yama-phase` fork can run its core CI, game-AI gates and trusted
action-pin audit on saget. These remain GitHub Actions workflows with their
existing test commands, test shards, WASM/Android checks and AI workloads.
`CI_LINUX_RUNNER` selects JSON runner labels; unset retains GitHub-hosted Linux.
The audit follows this fork's CI workflow ID, not the upstream repository's ID.
Production/lint/WASM/AI builds use stable Rust 1.98.1, matching the fork's
stable-build policy. The test archive and shards retain pinned nightly-2026-04-19
because probe-pin's existing libtest JSON tests require nightly; no tests are skipped.

From the `bryanroscoe/sagetserver` checkout:

```sh
python3 scripts/ci_runners.py register --repo yama-phase
python3 scripts/ci_runners.py switch saget --repo yama-phase --apply
python3 scripts/ci_runners.py switch github --repo yama-phase --apply
```

The saget value is `["self-hosted","Linux","X64","saget-ci"]`; hosted is
`["ubuntu-24.04"]`. Omit `--apply` to preview. Newly scheduled jobs use the
selection; assigned jobs finish on their original runner. Hosted private jobs
need available account minutes. Existing GitHub artifact/data-cache transfers
remain subject to the account's storage quota even with self-hosted compute.

The private `saget-yama-phase-ci` worker has its own container, network, runner
state, Cargo/tool/Android/Gradle caches and no Docker socket or production mounts.
Fork PR execution must remain disabled at repository level. Tools install into
writable project caches; checkout target symlinks preserve existing script/artifact
paths. Each Nextest shard extracts its archive under disposable RUNNER_TEMP,
so repeated shards cannot collide inside the persistent target cache. Rust target
cache stays on saget instead of being uploaded as GitHub Cargo caches. Sccache is
used only on hosted workers: a self-hosted daemon disconnect must not retry a long
compiler outside Cargo's concurrency limit. Hosted-only disk pruning never runs here.
Self-hosted Rust builds use one Cargo job: concurrent engine/test compilers can
exhaust the 12-GiB pool and starve the runner's job-lease renewal. Serializing
Cargo preserves the complete test suite.
Unmeasured CLI tools retain optimization level 1 and use 256 codegen units on
saget, matching the existing test profile's partition count. This splits the
CLI optimization units further; server, AI, benchmark and test profiles remain
unchanged. Private parse-baseline compiler identity includes this setting,
and cold baseline builds inherit it exactly.

All private workers share a four-CPU/12-GiB parent and two execution slots with
root-selected memory reservations. CI shares the production gate; Yama production
builds take it exclusively. Local job budgets include that wait. This changes throughput
and wall-clock timing; it does not reduce test/game counts or establish comparable
performance evidence between hosted and saget hardware. Temporary checkouts are
removed after jobs, while useful tools/dependency caches remain.

Enable `ci.yml`, `ai-gate.yml`, `action-pin-audit.yml` and the trusted
`coverage-parse-diff-comment.yml` after these adapters reach main. The comment
workflow runs on the selected CI backend, reads artifacts without checking out
PR code, and only posts evidence matching the current same-repository PR head.
All inherited release/deployment/maintenance workflows remain disabled and keep
their existing hosted routing; they are outside this CI migration. The two core-CI R2 publishing steps
require the additional `CI_ENABLE_PUBLISHING == 'true'` opt-in, which is unset
for this fork. Registration and CI validation do not authorize publishing,
application deployment, upstream changes, or moving Yama's pinned engine revision.

Private parse baselines do not use R2 or require `CI_ENABLE_PUBLISHING`. Main CI
saves coverage in this repository's private GitHub Actions cache, keyed by engine
source, exact MTGJSON input and compiler identity. PRs only restore these entries;
they cannot save a replacement main baseline. On a miss, eviction, or mismatched
metadata, CI exports the actual synthetic merge commit's base parent into
RUNNER_TEMP and generates its coverage with the same MTGJSON bytes and compiler.
The base build runs sequentially in the existing target cache, and its temporary
source/output tree is removed afterward. Head coverage remains untouched.
The resulting parse-diff artifact identifies both base and current PR head.
Failure to obtain private evidence fails the card-data job instead of reporting
an indefinitely pending public snapshot. A cold baseline adds a base engine build
and export; it uses existing CI capacity and private Actions storage quotas.

On saget, the first workflow step waits for CI capacity before checkout. The pre-job hook returns promptly so GitHub can report and cancel queued work. This step runs from RUNNER_TEMP, which exists before checkout, and verifies the runner supports the step-slot protocol. Install/adopt the reviewed sagetserver hooks before running these adapters. Hosted jobs skip this step.
The Rust summary job still reports failed dependencies, but skips a cancelled workflow so obsolete runs cannot hold the CI concurrency group.
AI gates also cancel superseded runs of the same PR. Manual and scheduled suites
use separate groups so a PR update does not displace those workloads.
