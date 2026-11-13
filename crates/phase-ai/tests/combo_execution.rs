//! Combo-execution bench: given an assembled combo of real cards, does the AI win with it?
//!
//! Each case puts a known combo's pieces where a player would have them (battlefield, or a
//! payoff in hand), opens the engine's loop detector (CR 732.2a, `LoopDetectionMode::Interactive`)
//! and lets the AI seats play P0's turn. The bench records whether P0 won, whether the engine
//! offered a loop shortcut, and what the seat did with it.
//!
//! The cases cover the shapes a combo takes:
//!
//! - a loop that kills by itself (Heliod + Walking Ballista),
//! - a loop that only makes a resource, beside a payoff that turns it into a win
//!   (Basalt Monolith + Power Artifact with Walking Ballista in hand),
//! - a loop whose every cycle makes a new object (Kiki-Jiki + Felidar Guardian),
//! - a two-card win with no loop (Thassa's Oracle + Demonic Consultation),
//! - a mandatory drain the seat only has to start (Exquisite Blood + Sanguine Bond).
//!
//! `#[ignore]`: this is a measurement, not a gate. It prints one row per case and difficulty.
//! Run it with
//! `cargo test -p phase-ai --test combo_execution -- --ignored --nocapture`.

use std::collections::{HashMap, HashSet};
use std::ops::ControlFlow;
use std::sync::Arc;

use engine::ai_support::AiDecisionContract;
use engine::analysis::loop_check::ShortcutResponse;
use engine::analysis::{IterationCount, WinKind};
use engine::database::CardDatabase;
use engine::game::bracket_estimate::CommanderBracketTier;
use engine::game::deck_loading::DeckEntry;
use engine::game::scenario::{GameScenario, P0, P1};
use engine::game::scenario_db::GameScenarioDbExt;
use engine::types::actions::GameAction;
use engine::types::card::CardFace;
use engine::types::counter::CounterType;
use engine::types::game_state::{GameState, LoopDetectionMode, PlayerDeckPool, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::phase::Phase;
use engine::types::player::PlayerId;
use engine::types::zones::Zone;
use phase_ai::auto_play::run_ai_action;
use phase_ai::config::{create_config_for_players, AiDifficulty, Platform};
use phase_ai::session::AiSession;
use rand::rngs::SmallRng;
use rand::SeedableRng;

/// Actions the two seats may take in the one turn before the bench gives up.
const MAX_ACTIONS: usize = 400;

/// The seed every row plays with.
const SEED: u64 = 7;

/// The opponent's life: Commander's starting total, so no case is won by ordinary damage.
const OPPONENT_LIFE: i32 = 40;

fn integration_card_db() -> CardDatabase {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../engine/tests/fixtures/integration_cards.json.gz");
    let file = std::fs::File::open(path).expect("integration fixture should open");
    let decoder = flate2::read::GzDecoder::new(std::io::BufReader::new(file));
    CardDatabase::from_export_reader(decoder).expect("integration fixture should load")
}

/// A board under construction: P0 to act in the first main phase, both libraries stocked.
struct Board<'a> {
    scenario: GameScenario,
    db: &'a CardDatabase,
    attachments: Vec<(ObjectId, ObjectId)>,
}

impl<'a> Board<'a> {
    fn new(db: &'a CardDatabase) -> Self {
        let mut scenario = GameScenario::new();
        scenario.at_phase(Phase::PreCombatMain);
        scenario.with_life(P0, OPPONENT_LIFE);
        scenario.with_life(P1, OPPONENT_LIFE);
        scenario.with_library_top(P0, &["Wastes"; 30]);
        scenario.with_library_top(P1, &["Wastes"; 30]);
        Board {
            scenario,
            db,
            attachments: Vec::new(),
        }
    }

    fn battlefield(&mut self, name: &str) -> ObjectId {
        self.scenario
            .add_real_card(P0, name, Zone::Battlefield, self.db)
    }

    fn hand(&mut self, name: &str) -> ObjectId {
        self.scenario.add_real_card(P0, name, Zone::Hand, self.db)
    }

    fn lands(&mut self, name: &str, count: usize) {
        for _ in 0..count {
            self.battlefield(name);
        }
    }

    fn attach(&mut self, attachment: ObjectId, host: ObjectId) {
        self.attachments.push((attachment, host));
    }

    fn build(self) -> GameState {
        let mut runner = self.scenario.build();
        let state = runner.state_mut();
        state.active_player = P0;
        state.priority_player = P0;
        for (attachment, host) in self.attachments {
            engine::game::effects::attach::attach_to(state, attachment, host);
        }
        runner.state().clone()
    }
}

/// Heliod, Sun-Crowned ("Whenever you gain life, put a +1/+1 counter on target creature or
/// enchantment you control." and "{1}{W}: Another target creature gains lifelink until end of
/// turn.") with Walking Ballista ("Remove a +1/+1 counter from this creature: It deals 1 damage
/// to any target."). The loop deals the damage itself.
fn heliod_ballista(db: &CardDatabase) -> GameState {
    let mut board = Board::new(db);
    board.battlefield("Heliod, Sun-Crowned");
    let ballista = board.battlefield("Walking Ballista");
    board
        .scenario
        .with_counter(ballista, CounterType::Plus1Plus1, 2);
    board.lands("Plains", 4);
    board.build()
}

/// Basalt Monolith ("{T}: Add {C}{C}{C}." and "{3}: Untap this artifact.") under Power Artifact
/// (its activated abilities cost {2} less) nets two colorless mana a cycle. Walking Ballista
/// ({X}{X}) in hand is the payoff that spends it.
fn basalt_power_artifact_ballista(db: &CardDatabase) -> GameState {
    let mut board = Board::new(db);
    let monolith = board.battlefield("Basalt Monolith");
    let artifact = board.battlefield("Power Artifact");
    board.attach(artifact, monolith);
    board.hand("Walking Ballista");
    board.lands("Island", 2);
    board.build()
}

/// Devoted Druid ("{T}: Add {G}." and "Put a -1/-1 counter on this creature: Untap this
/// creature.") beside Vizier of Remedies (one fewer -1/-1 counter is put on creatures you
/// control) untaps for free. Walking Ballista in hand is the payoff.
fn druid_vizier_ballista(db: &CardDatabase) -> GameState {
    let mut board = Board::new(db);
    board.battlefield("Devoted Druid");
    board.battlefield("Vizier of Remedies");
    board.hand("Walking Ballista");
    board.lands("Forest", 2);
    board.build()
}

/// Food Chain ("Exile a creature you control: Add X mana of any one color, where X is 1 plus the
/// exiled creature's mana value. Spend this mana only to cast creature spells.") with Eternal
/// Scourge ("You may cast this card from exile.") nets creature-only mana. Walking Ballista in
/// hand is a creature spell that spends it.
fn food_chain_scourge_ballista(db: &CardDatabase) -> GameState {
    let mut board = Board::new(db);
    board.battlefield("Food Chain");
    board.battlefield("Eternal Scourge");
    board.hand("Walking Ballista");
    board.lands("Forest", 2);
    board.build()
}

/// Kiki-Jiki, Mirror Breaker ("{T}: Create a token that's a copy of target nonlegendary creature
/// you control, except it has haste. Sacrifice it at the beginning of the next end step.") with
/// Felidar Guardian ("When this creature enters, you may exile another target permanent you
/// control, then return that card to the battlefield under its owner's control."). Every cycle
/// makes a hasty token; the win is the attack that follows.
fn kiki_felidar(db: &CardDatabase) -> GameState {
    let mut board = Board::new(db);
    board.battlefield("Kiki-Jiki, Mirror Breaker");
    board.battlefield("Felidar Guardian");
    board.lands("Mountain", 2);
    board.build()
}

/// Mikaeus, the Unhallowed (other non-Human creatures you control get +1/+1 and have undying)
/// with Triskelion (enters with three +1/+1 counters; "Remove a +1/+1 counter from this
/// creature: It deals 1 damage to any target."). Triskelion shoots the opponent, kills itself
/// and returns.
fn mikaeus_triskelion(db: &CardDatabase) -> GameState {
    let mut board = Board::new(db);
    board.battlefield("Mikaeus, the Unhallowed");
    board.battlefield("Triskelion");
    board.lands("Swamp", 2);
    board.build()
}

/// Phyrexian Altar ("Sacrifice a creature: Add one mana of any color.") sacrificing Gravecrawler
/// (cast from the graveyard while you control a Zombie) beside Walking Corpse, while Blood
/// Artist drains a player for each death.
fn altar_gravecrawler_blood_artist(db: &CardDatabase) -> GameState {
    let mut board = Board::new(db);
    board.battlefield("Phyrexian Altar");
    board.battlefield("Blood Artist");
    board.battlefield("Walking Corpse");
    board.battlefield("Gravecrawler");
    board.lands("Swamp", 2);
    board.build()
}

/// Thassa's Oracle ("When this creature enters, look at the top X cards of your library, where
/// X is your devotion to blue. … If X is greater than or equal to the number of cards in your
/// library, you win the game.") with Demonic Consultation, which exiles the library. No loop.
fn oracle_consultation(db: &CardDatabase) -> GameState {
    let mut board = Board::new(db);
    board.hand("Thassa's Oracle");
    board.hand("Demonic Consultation");
    board.lands("Island", 2);
    board.lands("Swamp", 1);
    board.build()
}

/// Exquisite Blood ("Whenever an opponent loses life, you gain that much life.") with Sanguine
/// Bond ("Whenever you gain life, target opponent loses that much life."). The loop is
/// mandatory once any life changes; Grizzly Bears attacking an empty board starts it.
fn exquisite_blood_sanguine_bond(db: &CardDatabase) -> GameState {
    let mut board = Board::new(db);
    board.battlefield("Exquisite Blood");
    board.battlefield("Sanguine Bond");
    board.battlefield("Grizzly Bears");
    board.lands("Swamp", 2);
    board.build()
}

type BoardFn = fn(&CardDatabase) -> GameState;

/// A case: its label, its board, and the cards a combo catalog would name for it (the
/// pieces and, where the loop only makes a resource, the payoff).
struct Case {
    name: &'static str,
    board: BoardFn,
    pieces: &'static [&'static str],
}

const CASES: &[Case] = &[
    Case {
        name: "heliod + ballista",
        board: heliod_ballista,
        pieces: &["Heliod, Sun-Crowned", "Walking Ballista"],
    },
    Case {
        name: "basalt + power artifact, ballista in hand",
        board: basalt_power_artifact_ballista,
        pieces: &["Basalt Monolith", "Power Artifact", "Walking Ballista"],
    },
    Case {
        name: "druid + vizier, ballista in hand",
        board: druid_vizier_ballista,
        pieces: &["Devoted Druid", "Vizier of Remedies", "Walking Ballista"],
    },
    Case {
        name: "food chain + scourge, ballista in hand",
        board: food_chain_scourge_ballista,
        pieces: &["Food Chain", "Eternal Scourge", "Walking Ballista"],
    },
    Case {
        name: "kiki-jiki + felidar guardian",
        board: kiki_felidar,
        pieces: &["Kiki-Jiki, Mirror Breaker", "Felidar Guardian"],
    },
    Case {
        name: "mikaeus + triskelion",
        board: mikaeus_triskelion,
        pieces: &["Mikaeus, the Unhallowed", "Triskelion"],
    },
    Case {
        name: "altar + gravecrawler + blood artist",
        board: altar_gravecrawler_blood_artist,
        pieces: &["Phyrexian Altar", "Gravecrawler", "Blood Artist"],
    },
    Case {
        name: "thassa's oracle + consultation",
        board: oracle_consultation,
        pieces: &["Thassa's Oracle", "Demonic Consultation"],
    },
    Case {
        name: "exquisite blood + sanguine bond",
        board: exquisite_blood_sanguine_bond,
        pieces: &["Exquisite Blood", "Sanguine Bond", "Grizzly Bears"],
    },
];

const DIFFICULTIES: &[AiDifficulty] = &[
    AiDifficulty::Easy,
    AiDifficulty::Medium,
    AiDifficulty::Hard,
    AiDifficulty::CEDH,
];

/// What one turn of play came to.
struct Report {
    outcome: String,
    actions: usize,
    offers: usize,
    /// What each offer stated, in the order the engine made them.
    offered: Vec<String>,
    declared: usize,
    declined: usize,
    opponent_life: i32,
    /// Both seats' actions other than passes, in order, with runs of the same action counted.
    line: Vec<(String, usize)>,
}

fn object_name(state: &GameState, id: ObjectId) -> String {
    state
        .objects
        .get(&id)
        .map(|object| object.name.clone())
        .unwrap_or_else(|| format!("{id:?}"))
}

/// A short label for an action, read against the state it was taken from.
fn label(state: &GameState, action: &GameAction) -> Option<String> {
    Some(match action {
        GameAction::PassPriority => return None,
        GameAction::ActivateAbility {
            source_id,
            ability_index,
            ..
        } => format!(
            "activate {}#{ability_index}",
            object_name(state, *source_id)
        ),
        GameAction::CastSpell { object_id, .. } => {
            format!("cast {}", object_name(state, *object_id))
        }
        GameAction::DeclareShortcut { count, .. } => format!("declare {count:?}"),
        GameAction::DeclineShortcut => "decline".to_string(),
        other => {
            let text = format!("{other:?}");
            text.chars().take(48).collect()
        }
    })
}

fn play_turn(mut state: GameState, difficulty: AiDifficulty) -> Report {
    state.loop_detection = LoopDetectionMode::Interactive;
    let turn = state.turn_number;
    let ai_players: HashSet<PlayerId> = [P0, P1].into_iter().collect();
    // Measurement mode: search is bounded by nodes and depth, never the clock, so a row is a
    // function of the build and the seed.
    let config = create_config_for_players(difficulty, Platform::Native, 2).into_measurement(SEED);
    let ai_configs: HashMap<PlayerId, _> =
        [(P0, config.clone()), (P1, config)].into_iter().collect();
    if difficulty == AiDifficulty::CEDH {
        // The combo policy reads the deck's declared bracket, not the difficulty. Declare
        // P0's deck cEDH for this row so the row measures that policy too.
        let entry = DeckEntry {
            card: CardFace::default(),
            count: 1,
        };
        state.deck_pools.clear();
        for (player, bracket_tier) in [
            (P0, CommanderBracketTier::Cedh),
            (P1, CommanderBracketTier::Core),
        ] {
            state.deck_pools.push(PlayerDeckPool {
                player,
                current_main: Arc::new(vec![entry.clone()]),
                bracket_tier,
                ..PlayerDeckPool::default()
            });
        }
    }
    let session = AiSession::arc_from_game(&state);
    let mut rng = SmallRng::seed_from_u64(SEED);

    let mut report = Report {
        outcome: String::new(),
        actions: 0,
        offers: 0,
        offered: Vec::new(),
        declared: 0,
        declined: 0,
        opponent_life: OPPONENT_LIFE,
        line: Vec::new(),
    };
    loop {
        if let WaitingFor::GameOver { winner } = &state.waiting_for {
            report.outcome = match winner {
                Some(player) if *player == P0 => "WON".to_string(),
                Some(_) => "lost".to_string(),
                None => "draw".to_string(),
            };
            break;
        }
        if state.turn_number > turn {
            report.outcome = "turn ended".to_string();
            break;
        }
        if report.actions >= MAX_ACTIONS {
            report.outcome = "action cap".to_string();
            break;
        }
        if let WaitingFor::LoopShortcut {
            predicted_winner,
            certificate,
            schema,
            ..
        } = &state.waiting_for
        {
            report.offers += 1;
            report.offered.push(format!(
                "winner {predicted_winner:?}, win {:?} at {:?}, suggests {:?}, bound {:?}, capacity {}, unbounded {:?}",
                certificate.win_kind,
                certificate.win_kind_at,
                schema.iteration_count,
                schema.measured_repetition_bound,
                schema.deliverable_capacity,
                certificate.unbounded,
            ));
        }
        let before = state.clone();
        match run_ai_action(&mut state, &ai_players, &ai_configs, &mut rng, &session) {
            ControlFlow::Continue(result) => {
                report.actions += 1;
                match &result.action {
                    GameAction::DeclareShortcut { .. } => report.declared += 1,
                    GameAction::DeclineShortcut => report.declined += 1,
                    _ => {}
                }
                if let Some(text) = label(&before, &result.action) {
                    match report.line.last_mut() {
                        Some((last, count)) if *last == text => *count += 1,
                        _ => report.line.push((text, 1)),
                    }
                }
            }
            ControlFlow::Break(stop) => {
                report.outcome = format!("stopped: {stop:?}").chars().take(80).collect();
                break;
            }
        }
    }
    report.opponent_life = state
        .players
        .iter()
        .find(|player| player.id == P1)
        .map_or(0, |player| player.life);
    report
}

/// The candidate cycles the engine's static ability graph names among P0's cards outside the
/// library: each cycle as its member cards and the axes it would drive up.
fn candidate_cycle_summary(state: &GameState) -> Vec<String> {
    use engine::analysis::ability_graph::{candidate_cycles_from_sources, AbilitySource};
    let objects: Vec<_> = state
        .objects
        .values()
        .filter(|object| object.controller == P0 && object.zone != Zone::Library)
        .collect();
    let sources: Vec<AbilitySource<'_>> = objects
        .iter()
        .map(|object| AbilitySource {
            mana_cost: &object.base_mana_cost,
            card_type: &object.base_card_types,
            abilities: &object.base_abilities,
            triggers: &object.base_trigger_definitions,
            replacements: &object.base_replacement_definitions,
            statics: &object.base_static_definitions,
        })
        .collect();
    candidate_cycles_from_sources(&sources)
        .iter()
        .map(|cycle| {
            let mut members: Vec<String> = cycle
                .members
                .iter()
                .map(|origin| format!("{}/{:?}", objects[origin.source].name, origin.slot))
                .collect();
            members.sort();
            members.dedup();
            format!("{} -> {:?}", members.join(" + "), cycle.unbounded)
        })
        .collect()
}

#[test]
#[ignore = "measurement, not a gate: prints one row per case and difficulty"]
fn assembled_combos_played_by_the_ai() {
    let db = integration_card_db();
    let mut won = 0usize;
    let mut total = 0usize;
    for case in CASES {
        let (name, board) = (case.name, case.board);
        println!("\n== {name}");
        for cycle in candidate_cycle_summary(&board(&db)) {
            println!("  cycle: {cycle}");
        }
        for difficulty in DIFFICULTIES {
            let report = play_turn(board(&db), *difficulty);
            total += 1;
            if report.outcome == "WON" {
                won += 1;
            }
            let line: Vec<String> = report
                .line
                .iter()
                .take(14)
                .map(|(text, count)| {
                    if *count > 1 {
                        format!("{text} x{count}")
                    } else {
                        text.clone()
                    }
                })
                .collect();
            let more = if report.line.len() > 14 { ", …" } else { "" };
            println!(
                "{difficulty:?}: {} | opponent {} | {} actions | offers {} declared {} declined {}\n    {}{more}",
                report.outcome,
                report.opponent_life,
                report.actions,
                report.offers,
                report.declared,
                report.declined,
                line.join(", "),
            );
            for offer in &report.offered {
                println!("    offer: {offer}");
            }
        }
    }
    println!("\nwon {won} of {total}");
}

// ---------------------------------------------------------------------------------------------
// Search prototype: told only which cards make the combo, can a plain search find a line the
// engine certifies as a win? It plays P0's choices on copies of the game, restricted at
// priority to the named cards, answers for the opponent, and takes a loop shortcut the engine
// offers. Nothing here knows what any card does.
// ---------------------------------------------------------------------------------------------

/// Reducer applies one search may spend.
const SEARCH_APPLIES: usize = 4000;

/// Wall-clock time one search may spend at one depth.
const SEARCH_TIME: std::time::Duration = std::time::Duration::from_secs(25);

/// Unbounded resources one line may take before it must win with what it has.
const STAGES: usize = 2;

/// Answers tried at one of P0's prompts.
const PROMPT_BRANCHES: usize = 5;

/// Repetitions taken from an offer that names no winner (unbounded mana, tokens, triggers).
const ADVANTAGE_REPETITIONS: u32 = 60;

struct Search<'a> {
    pieces: &'a [&'a str],
    applies: usize,
    /// Choice states already searched, with the most depth each was searched to.
    seen: HashMap<u64, usize>,
    offers: Vec<String>,
    trace: bool,
    /// The depth a search restarts with once it holds an unbounded resource.
    stage: usize,
    /// Whether the settle just run took an unbounded resource.
    resource_taken: bool,
    /// When this search gives up, whatever it has left to try.
    deadline: std::time::Instant,
    /// Whether the apply or time budget ended the search before it ran out of lines.
    cut: bool,
}

enum Settled {
    Won,
    Dead,
    /// P0 has a choice to make.
    Choice(Box<GameState>),
}

impl Search<'_> {
    fn apply(&mut self, state: &mut GameState, action: GameAction) -> bool {
        self.applies += 1;
        match engine::game::engine::apply_as_current_for_simulation(state, action) {
            Ok(_) => true,
            Err(error) => {
                if self.trace {
                    println!("      apply refused: {error:?}");
                }
                false
            }
        }
    }

    fn note_offer(&mut self, text: String) {
        if !self.offers.contains(&text) {
            self.offers.push(text);
        }
    }

    /// Plays everything that is not P0's own choice: the opponent passes and accepts, P0's
    /// stack resolves, and a shortcut offer is taken when it wins or makes a resource.
    fn settle(&mut self, mut state: GameState, line: &mut Vec<String>) -> Settled {
        loop {
            if self.applies >= SEARCH_APPLIES || std::time::Instant::now() >= self.deadline {
                self.cut = true;
                return Settled::Dead;
            }
            let action = match &state.waiting_for {
                WaitingFor::GameOver { winner } => {
                    return if *winner == Some(P0) {
                        Settled::Won
                    } else {
                        Settled::Dead
                    };
                }
                WaitingFor::LoopShortcut {
                    proposer,
                    predicted_winner,
                    certificate,
                    schema,
                    declaration,
                    ..
                } if *proposer == P0 => {
                    self.note_offer(format!(
                        "winner {predicted_winner:?}, win {:?}, suggests {:?}, capacity {}, unbounded {:?}",
                        certificate.win_kind,
                        schema.iteration_count,
                        schema.deliverable_capacity,
                        certificate.unbounded,
                    ));
                    // A win is taken to the end where the offer has no measured bound
                    // (the engine refuses "until lethal" against one that has), an unbounded
                    // resource for a fixed number of repetitions, and anything else for the
                    // count the engine suggests, with the engine's own suggested declaration.
                    let resource = certificate.win_kind == Some(WinKind::Advantage);
                    let count = if *predicted_winner == Some(P0) && !schema.is_bounded() {
                        IterationCount::UntilLethal
                    } else if resource {
                        IterationCount::Fixed(
                            ADVANTAGE_REPETITIONS.min(schema.deliverable_capacity),
                        )
                    } else {
                        schema.iteration_count.clone()
                    };
                    let declare = GameAction::DeclareShortcut {
                        count,
                        template: schema.suggestion(declaration.as_ref()).cloned(),
                    };
                    let text = label(&state, &declare).unwrap_or_default();
                    // CR 732.2b: a declaration the engine takes opens the other players'
                    // window or ends the game. One it refuses hands priority back and still
                    // returns `Ok`, so the result is read, not assumed.
                    let mut taken = state.clone();
                    let accepted = self.apply(&mut taken, declare)
                        && matches!(
                            taken.waiting_for,
                            WaitingFor::RespondToShortcut { .. } | WaitingFor::GameOver { .. }
                        );
                    if accepted {
                        line.push(text);
                        self.resource_taken |= resource;
                        state = taken;
                        continue;
                    }
                    line.push(format!("{text} refused, decline"));
                    GameAction::DeclineShortcut
                }
                WaitingFor::Priority { player } if *player == P0 => {
                    if state.stack.is_empty() {
                        return Settled::Choice(Box::new(state));
                    }
                    GameAction::PassPriority
                }
                WaitingFor::Priority { .. } => GameAction::PassPriority,
                other => {
                    let actors = other.acting_players();
                    if actors.contains(&P0) {
                        return Settled::Choice(Box::new(state));
                    }
                    let Some(actor) = actors.first().copied() else {
                        return Settled::Dead;
                    };
                    let candidates = AiDecisionContract::issue(&state, actor).candidates;
                    let accept = candidates.iter().find(|candidate| {
                        matches!(
                            candidate.action,
                            GameAction::RespondToShortcut {
                                response: ShortcutResponse::Accept
                            }
                        )
                    });
                    match accept.or(candidates.first()) {
                        Some(candidate) => candidate.action.clone(),
                        None => return Settled::Dead,
                    }
                }
            };
            if self.trace {
                let text: String = format!("{action:?}").chars().take(70).collect();
                let wait: String = format!("{:?}", state.waiting_for)
                    .chars()
                    .take(70)
                    .collect();
                println!("      settle: {text} at {wait}");
            }
            if !self.apply(&mut state, action) {
                return Settled::Dead;
            }
        }
    }

    fn is_piece(&self, state: &GameState, id: ObjectId) -> bool {
        state
            .objects
            .get(&id)
            .is_some_and(|object| self.pieces.contains(&object.name.as_str()))
    }

    /// P0's options at `state`, most promising first: at priority, casts and activations of
    /// the named cards; at a prompt, the opponent's face, then a named card, then the rest.
    fn options(&self, state: &GameState, holds_resource: bool) -> Vec<GameAction> {
        let at_priority = matches!(state.waiting_for, WaitingFor::Priority { .. });
        let mut actions: Vec<GameAction> = AiDecisionContract::issue(state, P0)
            .candidates
            .into_iter()
            .map(|candidate| candidate.action)
            .collect();
        if at_priority {
            // CR 605.3a: a mana ability may be activated whenever its controller has priority.
            // The AI's own candidates leave these out; a mana loop is made of them.
            for action in engine::game::mana_sources::activatable_mana_actions_for_player(state, P0)
            {
                if matches!(action, GameAction::ActivateAbility { .. })
                    && !actions.contains(&action)
                {
                    actions.push(action);
                }
            }
        }
        let mut ranked: Vec<((u8, u32), GameAction)> = actions
            .into_iter()
            .filter_map(|action| {
                let rank = match &action {
                    GameAction::PassPriority => return None,
                    GameAction::ActivateAbility { source_id, .. } if at_priority => {
                        if !self.is_piece(state, *source_id) {
                            return None;
                        }
                        1
                    }
                    GameAction::CastSpell { object_id, .. } if at_priority => {
                        if !self.is_piece(state, *object_id) {
                            return None;
                        }
                        // With a resource in hand the payoff comes first; before that, the
                        // loop does.
                        if holds_resource {
                            0
                        } else {
                            2
                        }
                    }
                    _ if at_priority => return None,
                    GameAction::ChooseTarget {
                        target: Some(engine::types::ability::TargetRef::Player(player)),
                    } => {
                        if *player == P1 {
                            0
                        } else {
                            3
                        }
                    }
                    GameAction::ChooseTarget {
                        target: Some(engine::types::ability::TargetRef::Object(id)),
                    } => {
                        if self.is_piece(state, *id) {
                            1
                        } else {
                            3
                        }
                    }
                    GameAction::SelectCards { cards }
                        if cards.iter().any(|id| self.is_piece(state, *id)) =>
                    {
                        1
                    }
                    GameAction::DecideOptionalEffect { accept: true } => 0,
                    GameAction::ChooseX { value } => {
                        return Some(((0, u32::MAX - *value), action));
                    }
                    _ => 2,
                };
                Some(((rank, 0), action))
            })
            .collect();
        ranked.sort_by_key(|(rank, _)| *rank);
        if !at_priority {
            ranked.truncate(PROMPT_BRANCHES);
        }
        ranked.into_iter().map(|(_, action)| action).collect()
    }

    fn dfs(
        &mut self,
        state: GameState,
        depth: usize,
        stages_left: usize,
        line: &mut Vec<String>,
    ) -> bool {
        let mark = line.len();
        self.resource_taken = false;
        let settled = self.settle(state, line);
        // Finding the loop and spending what it makes are two searches: the second starts
        // from the board the first one reached, with its own depth.
        let (depth, stages_left) = if self.resource_taken && stages_left > 0 {
            (self.stage, stages_left - 1)
        } else {
            (depth, stages_left)
        };
        let state = match settled {
            Settled::Won => return true,
            Settled::Dead => {
                if self.trace {
                    println!(
                        "      dead end after: {}",
                        line.last().cloned().unwrap_or_default()
                    );
                }
                line.truncate(mark);
                return false;
            }
            Settled::Choice(state) => *state,
        };
        // The serialized state: a prototype's key, slow, and blind to the loop detector's own
        // history, which is not serialized. Two boards that differ only in that history are
        // treated as one. Prompts are not keyed; each is reached from one keyed priority state.
        let hash = matches!(state.waiting_for, WaitingFor::Priority { .. }).then(|| {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            serde_json::to_vec(&state)
                .expect("a game state serializes")
                .hash(&mut hasher);
            hasher.finish()
        });
        if depth == 0
            || hash.is_some_and(|hash| {
                self.seen
                    .get(&hash)
                    .is_some_and(|searched| *searched >= depth)
            })
        {
            line.truncate(mark);
            return false;
        }
        if let Some(hash) = hash {
            self.seen.insert(hash, depth);
        }
        let options = self.options(&state, stages_left < STAGES);
        if self.trace {
            let wait: String = format!("{:?}", state.waiting_for)
                .chars()
                .take(60)
                .collect();
            println!(
                "{}[{depth}] {wait} | after: {} | options: {}",
                "  ".repeat(line.len().min(20)),
                line.last().cloned().unwrap_or_default(),
                options
                    .iter()
                    .map(|action| label(&state, action).unwrap_or_default())
                    .collect::<Vec<_>>()
                    .join(" / "),
            );
        }
        for action in options {
            if self.applies >= SEARCH_APPLIES || std::time::Instant::now() >= self.deadline {
                self.cut = true;
                break;
            }
            let step = line.len();
            line.push(label(&state, &action).unwrap_or_default());
            let mut next = state.clone();
            if self.apply(&mut next, action) && self.dfs(next, depth - 1, stages_left, line) {
                return true;
            }
            line.truncate(step);
        }
        line.truncate(mark);
        false
    }
}

#[test]
#[ignore = "measurement, not a gate: prints one row per case"]
fn assembled_combos_found_by_a_plain_search() {
    let db = integration_card_db();
    let mut found = 0usize;
    let only = std::env::var("COMBO_ONLY").ok();
    for case in CASES {
        if only.as_ref().is_some_and(|want| !case.name.contains(want)) {
            continue;
        }
        let mut state = (case.board)(&db);
        state.loop_detection = LoopDetectionMode::Interactive;
        let mut outcome = None;
        let mut applies = 0;
        let mut cut = false;
        let mut offers = Vec::new();
        for depth in [8, 14, 20] {
            let mut search = Search {
                pieces: case.pieces,
                applies: 0,
                seen: HashMap::new(),
                offers: Vec::new(),
                stage: depth,
                resource_taken: false,
                deadline: std::time::Instant::now() + SEARCH_TIME,
                cut: false,
                trace: std::env::var("COMBO_TRACE").is_ok_and(|want| case.name.contains(&want)),
            };
            let mut line = Vec::new();
            let won = search.dfs(state.clone(), depth, STAGES, &mut line);
            applies += search.applies;
            cut |= search.cut;
            for offer in search.offers {
                if !offers.contains(&offer) {
                    offers.push(offer);
                }
            }
            if won {
                outcome = Some((depth, line));
                break;
            }
        }
        println!("\n== {}", case.name);
        match &outcome {
            Some((depth, line)) => {
                found += 1;
                println!(
                    "FOUND at depth {depth}, {applies} applies\n    {}",
                    line.join(", ")
                );
            }
            None => {
                let why = if cut {
                    "the budget ran out"
                } else {
                    "every line to this depth was tried"
                };
                println!("not found, {applies} applies ({why})");
                for candidate in AiDecisionContract::issue(&state, P0).candidates {
                    let text: String = format!("{:?}", candidate.action)
                        .chars()
                        .take(150)
                        .collect();
                    println!(
                        "    root: {} | {text}",
                        label(&state, &candidate.action).unwrap_or_default()
                    );
                }
            }
        }
        for offer in &offers {
            println!("    offer: {offer}");
        }
    }
    println!("\nfound {found} of {}", CASES.len());
}
