# Upstream candidates

Status of each upstream candidate on `main`. Commits are named by subject, because SHAs change on every
rebase. Review findings came from a fresh-context `/review-engine-impl` pass on 2026-10-29. Each item
is a candidate for one PR, and goes only with Bryan's go-ahead, through upstream's `/engine-implementer`
pipeline (see [YAMA.md](YAMA.md)).

Status:
- **ready**: review clean, or only LOW notes listed here.
- **work**: a MED finding to fix first.
- **evidence**: needs measurements or a maintainer decision before a PR.

Verified on 2026-10-29 at the final stack on upstream 57ef03d (v0.119.0+): engine integration tests
13,948 passed / 0 failed / 11 ignored; phase-ai 3,212 passed / 0 failed / 24 ignored; `cargo fmt` and
Gate A pass; engine clippy (lib, bins, integration) shows only six lints, all on upstream lines. The
engine library tests do not fit the local 12 GiB compiler budget and have not run on this base; on the
previous base (24e8014) they passed when run in slices (one rewritten test, the Surveyor's Scope
coverage row, not yet run).

## Suggested first PRs

1. `fix(engine): stop the prospective payment search at its application allowance`: **ready**. Upstream
   still has this unbounded search: an exhausted budget wraps to `usize::MAX`, and one AI decision
   ran for over 20 minutes. Its test overflows without the fix.
2. `fix(engine): allow {T} beside a sacrifice in an {X} activation's remaining cost`: **ready**.
3. `fix(engine): a land's costed "Add one mana of any color" activates when chosen`: **ready**.
4. `fix(parser): an animation's "where X is" takes any where-X expression`: **ready**.
5. `fix(parser): "exile all the cards from your hand" is the whole hand`: **ready**.

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
| perf(ai): reuse the engine's named-choice validity proof in the planner's legality gate | evidence | Benchmarks on upstream's workload; not yet reviewed; its planner tests pass on this stack, not yet run without the guard |

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
| fix(parser): "a creature card put into a graveyard this way" reads the moved cards | work | One shared "this way" combinator instead of the fold-and-peel; runtime test |
| fix(engine): "each creature deals damage to itself" damages each source | ready | Needs a protocol bump |
| fix(parser): an unreadable "for each" cost reduction is a gap, not a count of everything | work | Reject a partly read remainder too; the `oracle_cost.rs` guard is unreachable |
| fix(parser): "enters with a counter for each ..." counts the right objects | work | Word-boundary split for the pair; drop the trailing-punctuation trim |
| fix(parser): "Turn it face up if it's face down. Then it deals damage" keeps its source | work | Runtime test for face-up and face-down |
| fix(engine): declined "up to one" targets leave "that creature" with no referent | work | Cast-path test with zero targets |
| fix(parser): "that spell's mana value" after "counter target spell" is the target | work | Add `Token { count }` to the quantity visitor; runtime test |
| fix(engine): a per-player sacrifice publishes what it sacrificed | work | Redesign onto upstream's `publish_tracked_set_with_causes` (add-to-set), dropping the `_for_cause` walkers |
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

## Other

| Commit | Status | Before a PR |
|---|---|---|
| fix(engine): build the forge feature against the current types | ready | LOW: `i32::try_from` for the sacrifice count |
