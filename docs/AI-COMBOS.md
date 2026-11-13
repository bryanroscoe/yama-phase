# Combos and deck plans in phase-ai

Status: started 2026-11-12. Built so far: the bench (step 1), the engine change and the
certified line search (steps 3 and 4, first version), and a host-supplied catalog with its
join against a deck (the first part of step 5). See "What is built" below.

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
- It is off by default (`LoopDetectionMode::Off`). `On` resolves only mandatory lethal
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
- `DeckFeatures` detects 24 structural axes of a deck, and `PlanSnapshot` is a mana-curve
  schedule built once per game. Nothing records how the deck wins or carries a goal from
  one decision to the next.
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

## What is built

- **Catalog** (`combo/catalog.rs`). `ComboCatalog` is a list of entries, each an id and
  the names of the cards that combine. `AiSession::with_combo_catalog` keeps, per seat,
  the entries whose cards are all in that seat's deck. `AiSession::with_combos` gives a
  seat its entries directly.
- **Certified lines** (`combo/certify.rs`). When every card of one of a seat's entries is
  somewhere the seat can use it, and it is the seat's main phase with an empty stack, the
  seat searches copies of the game for a line that wins. It tries the named cards' casts
  and activations, mana abilities included. Once the line holds an unbounded resource it
  tries the seat's other cards too, so the payoff need not be in the entry (a test finds
  Walking Ballista and Fireball in hand behind an entry naming only Basalt Monolith and
  Power Artifact). Only a line the reducer plays to a won game is kept. The seat then plays it one step per decision and drops it if the game
  stops matching what the line expected.
- **Engine** (`ai_support/context.rs`). The AI's decision contract admits two things the
  candidate sample leaves out: a permanent's mana ability at priority (CR 605.3a), checked
  against the engine's mana-source enumerator, and any repetition count from one to a
  counted loop offer's capacity (CR 732.2a), under the checks the declare handler makes of
  a count and with the engine's own suggested declaration. Neither is added to the
  candidates the AI scores, so ordinary play is unchanged.

A seat with no catalog, or a game without interactive loop detection, plays exactly as
before. So does a seat at the lowest difficulty (no play lookahead), a preset that samples
hidden zones, and a seat whose turn another player controls.

A search is bounded by a work budget and, outside measurement mode, by the clock (two
seconds, or the decision's own time budget if that is shorter). A seat starts at most three
searches a turn, and one that ran out of budget waits three of the seat's turns.

### With the combo catalogued

The bench again, with each case's cards given to the seat as a catalog entry
(`assembled_combos_played_by_the_ai_with_a_catalog`), four rows per case:

| Case | No catalog | Catalogued | Search |
|---|---|---|---|
| Heliod + Walking Ballista | 0 of 4 | 4 of 4 | about 40 ms |
| Basalt Monolith + Power Artifact, Ballista | 0 of 4 | 4 of 4 | about 340 ms |
| Devoted Druid + Vizier of Remedies, Ballista | 0 of 4 | 4 of 4 | about 340 ms |
| Food Chain + Eternal Scourge, Ballista | 0 of 4 | 4 of 4 | about 360 ms |
| Kiki-Jiki + Felidar Guardian | 0 of 4 | 0 of 4 | about 5 s, then gives up |
| Mikaeus + Triskelion | 0 of 4 | 0 of 4 | about 2 s, then gives up |
| Phyrexian Altar + Gravecrawler + Blood Artist | 0 of 4 | 0 of 4 | about 2 s, then gives up |
| Thassa's Oracle + Demonic Consultation | 0 of 4 | 0 of 4 | a few ms |

Times are one search on an M-series Mac, `tool` profile. A four-player game with Heliod and
Ballista is also won: the line finishes one opponent and carries on to the next (222 ms).

Eight tests in the same file are gates, not measurements: the damage loop at three
difficulties, the three mana loops, an unnamed payoff, the four-player game, two controls
(no catalog; loop detection off), and the two decision-contract rules on their own.

What the four misses need:

- **Kiki-Jiki + Felidar Guardian**: the search has to hand back to ordinary play after it
  takes the tokens, so the seat attacks with them.
- **Mikaeus + Triskelion, Altar + Gravecrawler + Blood Artist**: the lap is long and has
  many answers (which creature, which color, which target), and the search runs out of
  budget. Ranking a mana-color choice by what the named cards cost is the first thing to
  try.
- **Thassa's Oracle + Demonic Consultation**: the AI has no answer to the card-name prompt.

## Design

### 1. Deck plan

Built once per game and seat, beside `DeckFeatures` in `AiSession::from_game`. It is a new
type; the existing `plan::PlanSnapshot` is the curve schedule and keeps its name.

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

This needs an engine change as well. For an unbounded offer that names no winner, the
engine gives the AI one candidate, the decline (`ai_support/candidates.rs`), so today an AI
seat cannot accept an unbounded mana loop even when it wants to. Mana abilities on the
battlefield and in the command zone are also left out of the AI's candidates at priority.

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
2. **Loop detection on** for AI seats in Yama's lab, and the catalog passed in (a Yama
   change). Live Yama games already run with loop detection on.
3. **Finish and accept loops.** First version done 2026-11-13: a certified line is exempt
   from the caps, and the decision contract admits its mana abilities and counts.
4. **Certified line search** for an assembled route. First version done 2026-11-13; see
   the misses above.
5. **Deck plan**: catalog type and join done 2026-11-13. Still to do: the Spellbook
   export, typed outcomes, and payoffs. In the October 2026 Spellbook export about 20% of
   Commander-legal variants win by themselves; about 29% only make mana and about 51% only
   make triggers, tokens or cards, so most entries need a payoff from the deck beside them.
   The search now looks for one among the seat's other cards once it holds a resource
   (2026-11-13); nothing yet ranks a likely payoff above the rest of the hand.
6. **Goal**: tutors, mulligans, keeping and protecting pieces, score term.
7. **Roles and opponents.**
8. **Per-deck report** in Yama: for each deck, which of its combos the AI can run, and the
   engine gaps behind the ones it cannot.

## Baseline

2026-11-12, fork main `273ad56c8`, loop detection `Interactive`, one turn, opponent at 40
life with an empty board, measurement mode, seed 7. Four rows per case: Easy, Medium, Hard,
and cEDH with the deck declared cEDH so the combo policy is on.

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

The AI wins none of the 32 games that need it to do anything. That includes the cEDH rows
for the three combos the hand-written registry knows (Heliod, Kiki-Jiki, Thassa's Oracle).
What the runs show:

- **It never plays the first lap.** Each step of a loop looks bad alone (shoot your own
  creature, sacrifice a creature, untap something for no reason), so it is never chosen,
  and the engine has nothing to recognise. The only offers the engine made were to repeat
  an ordinary ability a few more times.
- **It cannot start a mana loop at all.** Mana abilities of permanents are left out of the
  actions the AI is given at priority; they are used only to pay for something. Basalt Monolith, Devoted
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
  first one reached. Without that split the prototype finished none of the three mana
  cases.
- After a token loop the right move is to hand back to ordinary play (attack), so taking an
  unbounded resource has to be a result the search can return, not only a win.
- Mikaeus + Triskelion and the Altar loop ran out of budget without a lethal offer. Each is
  either a search-order problem or an engine gap; both go on the bug list until known.

The prototype is test code. It keys visited states by serializing the game, which is slow
and leaves out the loop detector's own history (those fields are not serialized), so two
boards that differ only in that history count as one. Its lines repeat steps they do not
need. `planner::quick_state_hash`
was tried first and merged states that differ, so the production search needs its own key
(the loop detector's normalized state is the candidate).
