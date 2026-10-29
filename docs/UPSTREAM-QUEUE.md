# Upstream candidates

Status of each upstream candidate on `main`. Commits are named by subject, because SHAs change on every
rebase. Review findings came from a fresh-context `/review-engine-impl` pass on 2026-10-29; the prepared
branches below went through upstream's full `/engine-implementer` run and a Codex review. Each item
is a candidate for one PR, and goes only with Bryan's go-ahead, through upstream's `/engine-implementer`
pipeline (see [YAMA.md](YAMA.md)).

Status:
- **ready**: review clean, or only LOW notes listed here.
- **work**: a MED finding to fix first.
- **evidence**: needs measurements or a maintainer decision before a PR.

Verified on 2026-10-30 at the final stack on upstream df87a7c (`archive/pre-sync-20261030-2` keeps the
previous stack, on b3dc314; `archive/pre-sync-20261030` the one before, on 57ef03d): engine integration tests
14,239 passed / 0 failed / 11 ignored; phase-ai 3,235 passed / 0 failed / 24 ignored; `cargo fmt` and Gate A
pass. The engine and phase-ai tests ran on e1fbe5c; df87a7c adds only the new `state-codec` crate and
changes no engine or phase-ai file or their dependencies. Engine library tests were last run as one slice on the b3dc314 stack, because the full build doesn't
fit the local 13 GiB compiler budget: the 48 test modules the stack touches plus the shared test-support
modules, 10,644 passed / 0 failed / 4 ignored; they were not rerun on this base. Local runs on stable Rust
with one Cargo job; clippy was not run on this base.

## Suggested first PRs

1. `fix(engine): stop the prospective payment search at its application allowance`: **ready**. Upstream
   still has this unbounded search: an exhausted budget wraps to `usize::MAX`, and one AI decision
   ran for over 20 minutes. Its test overflows without the fix. Prepared; see below.
2. `fix(engine): allow {T} beside a sacrifice in an {X} activation's remaining cost`: **ready**.
3. `fix(engine): a land's costed "Add one mana of any color" activates when chosen`: **ready**.
4. `fix(parser): an animation's "where X is" takes any where-X expression`: **ready**.
5. `fix(parser): "exile all the cards from your hand" is the whole hand`: **ready**.

## Prepared upstream PR branches

Each branch in this repository is fresh `upstream/main` plus one commit, prepared through upstream's
`/engine-implementer` with proof logs. `main` carries the same commit unchanged (identical patch). Not yet
opened on phase-rs; that needs Bryan's go-ahead.

| Commit | Status | Branch | Preview PR |
|---|---|---|---|
| fix(engine): stop the prospective payment search at its application allowance | ready | `upstream-pr/payment-search-allowance` | #9 |
| fix(engine): finish a paused spell's resolution when its card is cast again during it | ready | `upstream-pr/creative-technique-recast` | #10 |
| fix(engine): untapping a land during payment is not a cast cost mutation (CR 733.1) | ready | `upstream-pr/cancel-cast-untapped-land` | #11 |
| fix(engine): a game that ends mid-resolution retires its pending choice (CR 104.1) | ready | `upstream-pr/game-over-clears-pending-choice` | #12 |

The payment-search commit replaces the fork's earlier version of the same fix. The game-over commit
on `main` is the reviewed branch's commit (identical patch), which replaced the fork's earlier version
in the second 2026-10-30 sync: it also retires the library search, look and reveal sessions a
finished resolution had opened.

## AI

| Commit | Status | Before a PR |
|---|---|---|
| perf(ai): cache a projection bail that the clock did not decide | ready | |
| fix(ai): classify named counters by the placing card's statics | ready | |
| fix(ai): animate a man-land only if it can and would attack or block | work | Size the body from X only when P/T is `CostXPaid`; let crew and tap-cost animations through |
| fix(ai): value "becomes a copy of target permanent" by what the copy gets | ready | |
| feat(ai): register the Thespian's Stage + Dark Depths combo line | ready | Optional; maintainers may not want card-named lines |
| fix(ai): score free-for-all attacks as a whole against the defender's block | work | Reuse `BlockStats`/`block_exchange` for gang blocks (indestructible, first strike) |
| fix(ai): no two-player race chumps with two or more opponents left | ready | |
| fix(ai): time board wipes by the leading opponent's board in a free-for-all | work | Apply the sweeper's filters to the leader's count too |

## Rules and parser

| Commit | Status | Before a PR |
|---|---|---|
| fix(engine): plan every unit of "N mana of any one color" | ready | LOW: reuse `mana_options_from_production`; the other multi-mana productions are still out of scope |
| fix(engine): tap a source before sacrificing it to its own activation | ready | LOW: also assert the tap event; `{Q}` is out of scope |
| feat(engine): parse "~ is every nonbasic land type" (Planar Nexus) | work | Fold into one `AddAllLandTypes { scope }` variant |
| fix(engine): creatures can't attack Jaces you control | work | Install as a rules-level `CantAttack` static, not a granted ability |
| fix(parser): "they may pay" asks and charges the player it names | ready | |
| fix(engine): don't re-stack a chain whose later target is already chosen | ready | LOW: move the guard below the modal router; add the 10-MV case |
| fix(engine): give every token of a repeated creation the follow-up instruction | work | Use `scan_at_word_boundaries` for the relative-clause cut |
| fix(parser): "<permanents> can't have <kind> counters put on them" | work | Add a runtime test (Melira) |
| fix(parser): "if you're the monarch" for activation costs and restrictions | work | Add a runtime test (Crown of Gondor equip) |
| fix(engine): a land's costed "Add one mana of any color" activates when chosen | ready | |
| feat(parser): command-zone cast counts for any player and any phrasing | work | "it's been cast" must count that commander only; add runtime tests |
| fix(parser): an animation's "where X is" takes any where-X expression | ready | |
| fix(parser): "exile all the cards from your hand" is the whole hand | ready | |
| fix(engine): decide layer-4 dependency on the board (CR 613.8) | evidence | Protected architecture: open an issue and get maintainer acceptance first; add an incremental-pass test; reduce per-pair state clones |
| fix(parser): "becomes a <creature type>" keeps the object's card types | ready | |
| fix(engine): "if that player / an opponent has more ... than you" reads the right players | ready | Needs a protocol bump |
| fix(engine): a consumer that relays the chain set doesn't block its producer | ready | |
| fix(parser): "a creature card put into a graveyard this way" reads the moved cards | work | One shared "this way" combinator instead of the fold-and-peel. Has a runtime test (Necromantic Selection) since the 2026-10-30 sync. It also binds Danse Macabre's singular die row, so it changes upstream's refusal pin for that row (`danse_put_into_zone_rows_refuse_honestly_while_neighbors_parse`, #9920); upstream defers this binding (unit 8e), so raise it with them first |
| fix(engine): "each creature deals damage to itself" damages each source | ready | Needs a protocol bump |
| fix(parser): an unreadable "for each" cost reduction is a gap, not a count of everything | work | Reject a partly read remainder too; the `oracle_cost.rs` guard is unreachable |
| fix(parser): "enters with a counter for each ..." counts the right objects | work | Word-boundary split for the pair; drop the trailing-punctuation trim |
| fix(parser): "Turn it face up if it's face down. Then it deals damage" keeps its source | work | Runtime test for face-up and face-down |
| fix(engine): declined "up to one" targets leave "that creature" with no referent | work | Cast-path test with zero targets |
| fix(parser): "that spell's mana value" after "counter target spell" is the target | work | Add `Token { count }` to the quantity visitor; runtime test |
| fix(parser): "if it's a <type>" gates on the object the sentence names | work | Drop the CR 109.2 citation in `assembly.rs`; runtime test |
| fix(parser): counter placements on mixed-type lists and "among those cards" | work | Clear the "that's a" guard only when that span was read |
| fix(parser): "for each opponent who ..." population predicates | work | Move the grammar out of frozen `oracle_quantity.rs`; drop CR 107.1; check every player anaphor |
| fix(parser): "They're <types> in addition to ..." and returned-card type lines | work | Require a battlefield-to-graveyard trigger for the dies binding |
| fix(parser): damage and life amounts from the event or the turn | ready | LOW: word-bound the recipient words |
| fix(engine): grant protection and hexproof from a non-persisting chosen color | evidence | Not yet reviewed; its 8 integration tests pass on this stack, not yet run without the fix |

## Performance

All five preserve behaviour, according to review. Each needs benchmark numbers on upstream's own
workload before a PR.

| Commit | Status | Before a PR |
|---|---|---|
| perf(engine): keep the off-battlefield static sources on the object store until it is next written | evidence | Benchmarks |
| perf(engine): skip the off-zone keyword query for objects no effect can give a companion | evidence | Benchmarks; compose `collect_continuous_effects` from the part helpers |
| perf(engine): share a scope's static-source gather when only the transients depend on context | evidence | Benchmarks; same composition |
| perf(engine): build a trigger source's snapshot only when a trigger can read it | evidence | Benchmarks |
| perf(engine): build the sorted card-name list once per database and share it | evidence | Benchmarks; reset the cache in `overlay_parser_backed_fixture_card` |

`keep the off-battlefield static sources ...` now feeds upstream's single off-zone walk
(`off_zone_static_effect_source_candidates`, #10405), and its cache field is boxed so `GameState` stays
within `game_state_size.rs`'s 14,848 B ceiling (unboxed it measured 14,864 B on aarch64-apple-darwin,
stable). `share a scope's static-source gather ...` and the companion pre-check of `skip the off-zone
keyword query ...` read the scope's off-zone source list from #10405, so a scope walks the object map once.
The `share a scope ...` commit also updates #10405's
`upkeep_collection_scans_off_zone_static_sources_once_per_keyword_scope` to expect the static part stored
once per scope and reused, and exactly one walk per scope.

## Other

| Commit | Status | Before a PR |
|---|---|---|
| fix(engine): build the forge feature against the current types | ready | LOW: `i32::try_from` for the sacrifice count |

## Dropped in the 2026-10-30 sync

Upstream made these redundant; the old commits stay reachable under `archive/pre-sync-20261030`.

- `fix(engine): a per-player sacrifice publishes what it sacrificed`: upstream #9920 (fac93d0) publishes a
  completed player-scope sacrifice's set through `publish_tracked_set_with_causes`
  (`publish_completed_sacrifice_clause_set`), the redesign this entry was waiting for. Keeping both would
  publish twice. The dropped commit's three integration tests (Syphon Flesh with and without a
  choice, Reign of the Pit) pass on the new base without it.
- `perf(ai): reuse the engine's named-choice validity proof in the planner's legality gate`: upstream
  #10417 (6102571) adds the same shortcut to `candidate_is_valid`, through
  `ai_support::structurally_valid_prompt_answer`.

## Second 2026-10-30 sync (onto df87a7c)

Nothing dropped: upstream's five new commits (fc76cb6, 014ed49, c7ab1d2, e1fbe5c, df87a7c) overlap no commit here, and
every commit other than the game-over fix rebased with an identical patch.
