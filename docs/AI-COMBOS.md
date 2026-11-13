# Combos and deck plans in phase-ai

Status: plan, started 2026-11-12. The bench and a search prototype exist (step 1); nothing
else here is built.

## Goal

The built-in AI should know what its deck is for. Before the first turn it works out how
the deck wins, which cards are the win conditions and which cards find or protect them.
During the game it works toward that, and when the pieces are there it plays the line and
wins. Infinite combos are the largest part of this, so they come first.

This is for the heuristic AI in `crates/phase-ai`, not the LLM seat in `crates/phase-llm`.

## What exists today

Engine (upstream work, in `crates/engine/src/analysis` and `crates/engine/src/game`):

- A loop detector. After a player plays one full repetition of a loop by hand, the engine
  recognises it, replays it on copies to confirm it, and offers a shortcut
  (`WaitingFor::LoopShortcut`, CR 732.2a). The player answers with a count, "until
  lethal", or a decline.
- It is off by default (`LoopDetectionMode::Off`). `On` only ends games on mandatory lethal
  loops. `Interactive` makes the offers.
- A corpus of 62 named combos with hand-written drivers (`analysis/corpus.rs`,
  `cargo combo-verify`). 24 are driven, the rest are gated on a card or deferred by class.

AI (`crates/phase-ai`):

- `policies/loop_shortcut.rs` answers an offer once the engine makes one. It takes a
  winning shortcut and refuses a losing one.
- `combo/` holds four hand-written, name-matched lines. `ComboLinePolicy` uses them only
  for a deck declared cEDH, and only when every piece is already present and affordable.
- `reach/` finds lethal burn: it proposes a set of sources, plays them on a copy of the
  game, and commits only a line the engine confirmed. This is the model for step 4.
- `DeckFeatures` detects 24 structural axes of a deck. There is no plan that lasts beyond
  one decision.
- Two caps stop runaway play: four activations of one ability per turn and three casts of
  one card per turn (`search.rs`). They also stop a seat from playing the first repetition
  of a real loop.

## The bench

`crates/phase-ai/tests/combo_execution.rs` puts an assembled combo of real cards in front of
the AI with loop detection on and records whether it wins. It is the measure for every step
below: a step is done when the cases it targets turn from lost to won without others
regressing.

```sh
cargo +stable test --profile tool -p phase-ai --test combo_execution -- --ignored --nocapture
```

Baseline, 2026-11-12, fork main `273ad56c8`: see "Baseline" at the end of this file.

## Design

### 1. Deck plan

Built once per game and seat, beside `DeckFeatures` in `AiSession::from_game`.

- **Combos in this deck.** A join of the deck's cards against a combo catalog. Each entry
  has its pieces, where each piece must be (battlefield, hand, graveyard, command zone), the
  mana it needs, and what it produces.
- **What each combo produces**, as a typed outcome: wins outright, unbounded damage,
  unbounded mana (which colors, any restriction), unbounded tokens, unbounded enters or
  dies triggers, unbounded draw or mill, a lock. An outcome that is not a win names the
  payoffs in the deck that turn it into one (an X spell for unbounded mana, a drain trigger
  for unbounded deaths).
- **Card roles**: combo piece, payoff, tutor, protection, engine, ramp, removal. Detected
  from the parsed card, as `DeckFeatures` does. No card names in code.
- **Win routes**, ranked: each combo, then the deck's fair plans (commander damage, wide
  board, drain, mill, alternate win cards).

The catalog is data, not code. The host supplies it, the way it supplies card data. The
format follows Commander Spellbook's bulk export (pieces by Oracle id, zones, mana,
produced features). phase-ai defines the typed catalog and the join; it ships no list.

### 2. A goal that lasts

The seat keeps a current route (the nearest combo by missing pieces and mana, or a fair
plan) in the session, and re-ranks when the board changes. The route feeds decisions the AI
already makes:

- tutors fetch the missing piece (replacing the registry lookup in `policies/tutor.rs`),
- mulligans keep hands with pieces or tutors,
- pieces are not thrown away as blockers, discards or sacrifices,
- protection is held for the turn the combo goes off,
- the position score counts progress toward the route, so search sees it too.

### 3. Recognising a loop the engine can shortcut

Loop detection goes on for AI seats (`Interactive`). The two caps stop counting a repetition
the engine is tracking as a candidate loop, so a seat can finish the first lap and receive
the offer. A resource-only offer (unbounded mana, tokens, triggers) is accepted when the
deck plan names a payoff the seat can then use, for a count that covers it.

### 4. Playing the line

When a route's pieces are in place, the seat searches for the line on a copy of the game:

1. Limit the moves to the route's pieces, mana, and the payoff.
2. Play forward until the engine offers a shortcut, or the game is won.
3. If the offer is a win, commit that exact line at the real decision, as `reach/` does.
4. If the offer is a resource, take it and search again for the payoff.

The engine certifies every step, so a combo the engine cannot yet run is never attempted in
a real game. It shows up as a failed case instead, which is where the bug list comes from.

### 5. Opponents

A seat reads the same roles off the cards an opponent has shown. A visible combo piece or
engine is a removal priority and raises that opponent's threat, replacing "attack whoever
is easiest".

## Where the data comes from

- **Commander Spellbook** (about 115,000 variants in the October 2026 export): the catalog.
  Its site and backend code are MIT. The combo data carries no stated licence that we have
  found; ask before shipping it inside another project. Yama already uses it, with credit.
- **phase's own corpus** (62 rows, MIT): the only source with lines a machine can run.
- **Found by the engine**: `analysis::ability_graph::candidate_cycles` proposes candidate
  cycles from the cards alone and the loop detector confirms them. Slow, but it needs no
  outside data. Worth trying once step 4 exists, since it is the same search.
- **Forge's combo lists** are GPL and are not used. Whoever has read Forge's source does
  not write this code (see `YAMA.md`).

## Sending it upstream

Everything except the Spellbook file is general: the bench, the catalog type and join, the
goal, the cap change, the certified line search. Each is written as an upstream candidate.
The host passes the catalog in, so upstream can choose its own source. Upstream issue
`#10041` (the AI declines a mana loop it needs) is step 3's first case. Nothing is sent
without Bryan's go-ahead.

## Steps

1. **Bench and baseline.** Done 2026-11-12.
2. **Loop detection on** for AI seats in Yama's lab and games (a Yama change).
3. **Finish and accept loops**: the cap change, and resource offers with a payoff.
4. **Certified line search** for an assembled route.
5. **Deck plan**: catalog type, Spellbook join, typed outcomes, payoffs.
6. **Goal**: tutors, mulligans, keeping and protecting pieces, score term.
7. **Roles and opponents.**
8. **Per-deck report** in Yama: for each deck, which of its combos the AI can run, and the
   engine gaps behind the ones it cannot.

## Baseline

2026-11-12, fork main `273ad56c8`, loop detection `Interactive`, one turn, opponent at 40
life with an empty board. Four difficulties per case (Easy, Medium, Hard, cEDH).

| Case | Shape | AI wins |
|---|---|---|
| Heliod + Walking Ballista | loop deals the damage | 0 of 4 |
| Basalt Monolith + Power Artifact, Ballista in hand | mana loop and a payoff | 0 of 4 |
| Devoted Druid + Vizier of Remedies, Ballista in hand | mana loop and a payoff | 0 of 4 |
| Food Chain + Eternal Scourge, Ballista in hand | restricted mana loop and a payoff | 0 of 4 |
| Kiki-Jiki + Felidar Guardian | new object each lap, then attack | 0 of 4 |
| Mikaeus + Triskelion | piece dies and returns each lap | 0 of 4 |
| Phyrexian Altar + Gravecrawler + Blood Artist | sacrifice loop with a drain | 0 of 4 |
| Thassa's Oracle + Demonic Consultation | two-card win, no loop | 0 of 4 |
| Exquisite Blood + Sanguine Bond (control) | mandatory once started | 4 of 4 |

The AI wins none of the 32 games that need it to do anything. What the runs show:

- **It never plays the first lap.** Each step of a loop looks bad alone (shoot your own
  creature, sacrifice a creature, untap something for no reason), so it is never chosen,
  and the engine has nothing to recognise. The only offers the engine made were to repeat
  an ordinary ability a few more times.
- **It cannot start a mana loop at all.** Mana abilities are left out of the actions the AI
  is given at priority; they are used only to pay for something. Basalt Monolith, Devoted
  Druid, Food Chain and Phyrexian Altar loops all begin with one.
- **It spends the payoff early.** With Ballista in hand it casts it for X=1 or 2 first.
- **Demonic Consultation stops the seat.** After casting it the AI returns no action
  (`ChooseActionNone`); the card-name prompt has no answer the AI can give.
- **The static ability graph sees little of this.** `candidate_cycles` names a cycle for
  Basalt Monolith and fragments for three other cases, and nothing for the rest, so it
  cannot be the only guide to which cards combine. A catalog is needed.

### What a guided search finds

The same file holds a prototype (`assembled_combos_found_by_a_plain_search`). It is told
only the names of the combo's cards, tries their casts and activations on copies of the
game (mana abilities included), answers prompts, and accepts what the engine offers. It
knows nothing about what a card does.

| Case | Prototype | Engine steps | What the engine offered |
|---|---|---|---|
| Heliod + Walking Ballista | certified win | 32 | a lethal count (39 repetitions) |
| Basalt Monolith + Power Artifact, Ballista | certified win | 546 | unbounded colorless mana, then a lethal count |
| Devoted Druid + Vizier of Remedies, Ballista | certified win | 582 | unbounded green mana, then a lethal count |
| Food Chain + Eternal Scourge, Ballista | certified win | 1,013 | unbounded mana of each color, then a lethal count |
| Kiki-Jiki + Felidar Guardian | no win | 172 | unbounded tokens; the win is an attack, which the prototype does not make |
| Mikaeus + Triskelion | no win | 8,908 | short counts and an unbounded counter loop, no lethal offer |
| Phyrexian Altar + Gravecrawler + Blood Artist | no win | 10,169 | short counts only |
| Thassa's Oracle + Demonic Consultation | no win | 60 | nothing: the card-name prompt has no answer |
| Exquisite Blood + Sanguine Bond (control) | no win | 0 | nothing: the prototype does not attack |

Four of the eight real combos are found and certified from the card names alone. What this
settles for the design:

- The engine already certifies these loops once the laps are played, including the
  two-stage ones (make the mana, then spend it). The AI's missing part is the search.
- The search has to be allowed to activate mana abilities at priority.
- A resource loop and its payoff are two searches. The second starts from the board the
  first one reached. Without that split the prototype did not finish either mana case.
- After a token loop the right move is to hand back to ordinary play (attack), so taking an
  unbounded resource has to be a result the search can return, not only a win.
- Mikaeus + Triskelion and the Altar loop need a look at why no lethal offer appears. Each
  is either a search-order problem or an engine gap; both go on the bug list until known.

The prototype is test code. It keys visited states by serializing the whole game, which is
exact and slow, and its lines repeat steps they do not need. `planner::quick_state_hash`
was tried first and merged states that differ, so the production search needs its own key
(the loop detector's normalized state is the candidate).
