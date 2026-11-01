# Performance baseline acceptance, 1 November 2026

Bryan explicitly accepted the existing fork costs on 1 November 2026:
“Accept existing costs and refresh the baseline.” This approves the measured
baseline below. It does not establish the historical cause of the increases,
waive future gate failures, or change the baseline-refresh policy for later work.

The four failures against the previous baseline were:

| Counter | Previous baseline | Previous limit | Accepted measurement |
| --- | ---: | ---: | ---: |
| `attackable_player_sweeps` | 1,034 | 1,149 | 1,230 |
| `layers_full_eval` | 6,023 | 6,388 | 6,504 |
| `legend_rule_mode_gate_scans` | 11,283 | 11,911 | 12,191 |
| `sba_battlefield_snapshot_builds` | 10,920 | 11,530 | 11,828 |

All 45 measured counters match the earlier fork-main control and the acceptance
candidate. The acceptance fixes add no measured counter cost in these scenarios.
This is not a claim about every workload or about wall-clock equivalence.

## Reproducibility and budget evidence

- [Hosted validation run 41320157438](https://github.com/bryanroscoe/yama-phase/actions/runs/41320157438),
  artifact `13089039106`, source `6ce470a7e250f17efddc959d82073659cfde1f67`.
- All 25 validation reports have identical 45 counters; all comparison-band
  checks pass and the margin report has zero over-margin counters.
- Cold isolated `server-release` build: 472 seconds. Slowest complete
  median-of-five run: 52 seconds. Budget: `472 + 2.5 * 52 = 602 < 1500` seconds.
- Rust `1.97.0-nightly (0febdbab2 2026-04-18)`, Linux x86-64.
- Engine tree: `f7a9a831269546b6b3482d6d3785ab674de1c062`.
- AI tree before this baseline refresh: `16d8fe498dd9ed85c839786d0ca0d9e905b19a17`.
- Full generated card-data SHA-256:
  `6a8f7bf01044aa0aaee439c5ea7f197a9d36cef99161aea3828b47ee5f96feb1`.

The committed baseline is copied byte-for-byte from that generated report,
including its original `git_sha` and timing field. Later workflow-only commits
preserve its engine, AI source and build inputs. Independent review verified the
reports, source identity, margin and budget before the explicit acceptance.

## Unchanged gate contract

Schema 6; 46-card scenario fingerprint
`cea608c514890427e9d9ddc39536155d56f8ea31`; seed 2654435769; action cap 3000;
five cold-process samples; `red-mirror`, `affinity-mirror`, `enchantress-mirror`.
The `1.05x + 64` comparison band, 50% headroom margin, 25 validation runs and
strict 1500-second build-plus-gate acceptance budget remain unchanged.

The complete generated baseline retains counter decreases as well as increases.
Fresh normal CI must validate the refreshed baseline before PR #1 merges.
