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
use engine::types::game_state::{
    CastPaymentMode, GameState, LoopDetectionMode, PlayerDeckPool, WaitingFor,
};
use engine::types::identifiers::ObjectId;
use engine::types::phase::Phase;
use engine::types::player::PlayerId;
use engine::types::zones::Zone;
use phase_ai::auto_play::run_ai_action;
use phase_ai::combo::{combo_work_due, rehearse, CatalogCombo, Rehearsal};
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
        Self::for_players(db, 2)
    }

    /// A board for `players` seats, each at Commander's starting life.
    fn for_players(db: &'a CardDatabase, players: u8) -> Self {
        let mut scenario = if players == 2 {
            GameScenario::new()
        } else {
            GameScenario::new_n_player(players, SEED)
        };
        scenario.at_phase(Phase::PreCombatMain);
        for seat in (0..players).map(PlayerId) {
            scenario.with_life(seat, OPPONENT_LIFE);
            scenario.with_library_top(seat, &["Wastes"; 30]);
        }
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

    fn library(&mut self, name: &str) -> ObjectId {
        self.scenario
            .add_real_card(P0, name, Zone::Library, self.db)
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

/// Let the AI seats play P0's turn. `catalogued` is the combo P0's session is told its deck
/// holds, by card name; empty leaves the session without a catalog.
fn play_turn(
    mut state: GameState,
    difficulty: AiDifficulty,
    loops: LoopDetectionMode,
    catalogued: &[&str],
) -> Report {
    state.loop_detection = loops;
    let turn = state.turn_number;
    let ai_players: HashSet<PlayerId> = state.players.iter().map(|seat| seat.id).collect();
    // Measurement mode: search is bounded by nodes and depth, never the clock, so a row is a
    // function of the build and the seed.
    let seats = u8::try_from(ai_players.len()).expect("a table of a few seats");
    let config =
        create_config_for_players(difficulty, Platform::Native, seats).into_measurement(SEED);
    let ai_configs: HashMap<PlayerId, _> = ai_players
        .iter()
        .map(|seat| (*seat, config.clone()))
        .collect();
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
    let mut session = AiSession::from_game(&state);
    if !catalogued.is_empty() {
        session = session.with_combos(
            P0,
            vec![CatalogCombo {
                id: "bench".to_string(),
                cards: catalogued.iter().map(|card| card.to_string()).collect(),
            }],
        );
    }
    let session = Arc::new(session);
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
    print_bench(false);
}

/// The same bench with each case's cards given to P0's session as a catalogued combo.
#[test]
#[ignore = "measurement, not a gate: prints one row per case and difficulty"]
fn assembled_combos_played_by_the_ai_with_a_catalog() {
    print_bench(true);
}

fn print_bench(with_catalog: bool) {
    let db = integration_card_db();
    let mut won = 0usize;
    let mut total = 0usize;
    let only = std::env::var("COMBO_ONLY").ok();
    for case in CASES {
        if only.as_ref().is_some_and(|want| !case.name.contains(want)) {
            continue;
        }
        let (name, board) = (case.name, case.board);
        println!("\n== {name}");
        for cycle in candidate_cycle_summary(&board(&db)) {
            println!("  cycle: {cycle}");
        }
        for difficulty in DIFFICULTIES {
            let catalogued: &[&str] = if with_catalog { case.pieces } else { &[] };
            let report = play_turn(
                board(&db),
                *difficulty,
                LoopDetectionMode::Interactive,
                catalogued,
            );
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
// Gates: a seat told its deck holds a combo wins with it once the pieces are assembled.
// ---------------------------------------------------------------------------------------------

fn case(name: &str) -> &'static Case {
    CASES
        .iter()
        .find(|case| case.name == name)
        .expect("a bench case by that name")
}

/// A loop that deals the damage itself, at every searching and non-searching difficulty the
/// lab runs.
#[test]
fn a_seat_wins_with_a_catalogued_damage_loop() {
    let db = integration_card_db();
    let heliod = case("heliod + ballista");
    for difficulty in [AiDifficulty::Easy, AiDifficulty::Medium, AiDifficulty::Hard] {
        let report = play_turn(
            (heliod.board)(&db),
            difficulty,
            LoopDetectionMode::Interactive,
            heliod.pieces,
        );
        assert_eq!(report.outcome, "WON", "{difficulty:?}: {:?}", report.line);
    }
}

/// A loop that only makes mana, beside a payoff in hand: the seat makes the mana, casts the
/// payoff with it and wins. Three sources of the mana: an artifact that untaps itself, a
/// creature that untaps itself, and a creature exiled and cast again.
#[test]
fn a_seat_wins_with_a_catalogued_mana_loop_and_its_payoff() {
    let db = integration_card_db();
    for name in [
        "food chain + scourge, ballista in hand",
        "basalt + power artifact, ballista in hand",
        "druid + vizier, ballista in hand",
    ] {
        let case = case(name);
        let report = play_turn(
            (case.board)(&db),
            AiDifficulty::Medium,
            LoopDetectionMode::Interactive,
            case.pieces,
        );
        assert_eq!(report.outcome, "WON", "{name}: {:?}", report.line);
    }
}

/// A catalog entry names the cards that make the loop, not what spends it. With the mana
/// loop catalogued and the payoff not named, the seat still finds the payoff in its hand:
/// a creature whose counters are its damage, or a sorcery with X in its cost.
#[test]
fn a_seat_finds_a_payoff_the_catalog_does_not_name() {
    let db = integration_card_db();
    for payoff in ["Walking Ballista", "Fireball"] {
        let mut board = Board::new(&db);
        let monolith = board.battlefield("Basalt Monolith");
        let artifact = board.battlefield("Power Artifact");
        board.attach(artifact, monolith);
        board.hand(payoff);
        board.hand("Grizzly Bears");
        board.lands("Mountain", 2);
        let report = play_turn(
            board.build(),
            AiDifficulty::Medium,
            LoopDetectionMode::Interactive,
            &["Basalt Monolith", "Power Artifact"],
        );
        assert_eq!(report.outcome, "WON", "{payoff}: {:?}", report.line);
    }
}

/// CR 104.2a: with three opponents the game is won only when all three are out. The loop is
/// offered against one opponent at a time, and the line carries on to the next.
#[test]
fn a_seat_wins_a_four_player_game_with_a_catalogued_damage_loop() {
    let db = integration_card_db();
    let mut board = Board::for_players(&db, 4);
    board.battlefield("Heliod, Sun-Crowned");
    let ballista = board.battlefield("Walking Ballista");
    board
        .scenario
        .with_counter(ballista, CounterType::Plus1Plus1, 2);
    board.lands("Plains", 4);
    let report = play_turn(
        board.build(),
        AiDifficulty::Medium,
        LoopDetectionMode::Interactive,
        &["Heliod, Sun-Crowned", "Walking Ballista"],
    );
    assert_eq!(report.outcome, "WON", "{:?}", report.line);
}

/// The catalog is what changes the play: the same board with no catalog is not won.
#[test]
fn a_seat_without_a_catalog_does_not_find_the_loop() {
    let db = integration_card_db();
    let heliod = case("heliod + ballista");
    let report = play_turn(
        (heliod.board)(&db),
        AiDifficulty::Medium,
        LoopDetectionMode::Interactive,
        &[],
    );
    assert_ne!(report.outcome, "WON", "{:?}", report.line);
}

/// CR 732.2a: the line ends in a shortcut only the interactive detector offers, so with the
/// detector off the seat does not start one.
#[test]
fn a_seat_does_not_search_with_loop_detection_off() {
    let db = integration_card_db();
    let heliod = case("heliod + ballista");
    let report = play_turn(
        (heliod.board)(&db),
        AiDifficulty::Medium,
        LoopDetectionMode::Off,
        heliod.pieces,
    );
    assert_ne!(report.outcome, "WON", "{:?}", report.line);
}

// ---------------------------------------------------------------------------------------------
// The deck plan: a seat rehearses its combos to learn which it can win with.
// ---------------------------------------------------------------------------------------------

/// A game with nothing drawn yet: the named cards and four of each named land are in P0's
/// library, P0 to act in its first main phase.
fn undrawn(db: &CardDatabase, cards: &[&str], lands: &[&str]) -> GameState {
    let mut board = Board::new(db);
    for card in cards {
        board.library(card);
    }
    for land in lands {
        for _ in 0..4 {
            board.library(land);
        }
    }
    let mut state = board.build();
    state.loop_detection = LoopDetectionMode::Interactive;
    state
}

fn combo(id: &str, cards: &[&str]) -> CatalogCombo {
    CatalogCombo {
        id: id.to_string(),
        cards: cards.iter().map(|card| card.to_string()).collect(),
    }
}

/// With every card still in the library, a rehearsal puts a combo out on a copy of the game
/// and plays it. A loop that deals the damage itself is won as it stands. A mana loop is
/// won once the deck's X spell is tried with it, and that spell is named. A combo this AI
/// cannot play is not won: Thassa's Oracle with Demonic Consultation, set out with the
/// Oracle in play and then with both in hand.
#[test]
fn a_rehearsal_shows_which_combos_a_seat_can_win_with() {
    let db = integration_card_db();
    let state = undrawn(
        &db,
        &[
            "Heliod, Sun-Crowned",
            "Walking Ballista",
            "Basalt Monolith",
            "Power Artifact",
            "Thassa's Oracle",
            "Demonic Consultation",
        ],
        &["Plains", "Island", "Swamp"],
    );

    assert_eq!(
        rehearse(
            &state,
            P0,
            &combo("damage", &["Heliod, Sun-Crowned", "Walking Ballista"])
        ),
        Some(Rehearsal::Wins { with: Vec::new() })
    );
    assert_eq!(
        rehearse(
            &state,
            P0,
            &combo("mana", &["Basalt Monolith", "Power Artifact"])
        ),
        Some(Rehearsal::Wins {
            with: vec!["Walking Ballista".to_string()]
        })
    );
    assert_eq!(
        rehearse(
            &state,
            P0,
            &combo("oracle", &["Thassa's Oracle", "Demonic Consultation"])
        ),
        Some(Rehearsal::NoWin)
    );
    // A card the seat does not own, and a position that is not the seat's to act in.
    assert_eq!(
        rehearse(&state, P0, &combo("absent", &["Food Chain"])),
        None
    );
    assert_eq!(
        rehearse(
            &state,
            P1,
            &combo("damage", &["Heliod, Sun-Crowned", "Walking Ballista"])
        ),
        None
    );
}

/// A seat rehearses one combo a turn at its first decision in its main phase, and keeps
/// what it showed. Until then the host is told there is combo work to do here; after it,
/// not again this turn, though a second combo is still to be rehearsed.
#[test]
fn a_seat_rehearses_a_combo_at_its_first_main_phase_decision() {
    let db = integration_card_db();
    let mut state = undrawn(
        &db,
        &["Heliod, Sun-Crowned", "Walking Ballista"],
        &["Plains", "Plains"],
    );
    let session = Arc::new(AiSession::from_game(&state).with_combos(
        P0,
        vec![
            combo("damage", &["Heliod, Sun-Crowned", "Walking Ballista"]),
            combo("second", &["Heliod, Sun-Crowned", "Plains"]),
        ],
    ));
    let config =
        create_config_for_players(AiDifficulty::Medium, Platform::Native, 2).into_measurement(SEED);
    assert!(session.rehearsals(P0).is_empty());
    assert!(combo_work_due(&state, &session, P0, &config));
    assert!(!combo_work_due(&state, &session, P1, &config));
    // A difficulty that never looks ahead never rehearses.
    let no_lookahead = create_config_for_players(AiDifficulty::VeryEasy, Platform::Native, 2);
    assert!(!no_lookahead.play_lookahead);
    assert!(!combo_work_due(&state, &session, P0, &no_lookahead));

    let ai_players: HashSet<PlayerId> = [P0].into_iter().collect();
    let ai_configs: HashMap<PlayerId, _> = [(P0, config.clone())].into_iter().collect();
    let mut rng = SmallRng::seed_from_u64(SEED);
    assert!(matches!(
        run_ai_action(&mut state, &ai_players, &ai_configs, &mut rng, &session),
        ControlFlow::Continue(_)
    ));

    assert_eq!(
        session.rehearsals(P0),
        vec![("damage".to_string(), Rehearsal::Wins { with: Vec::new() })]
    );
    assert!(!combo_work_due(&state, &session, P0, &config));

    // A host that kept the result hands it to the next game, and nothing is rehearsed.
    let fresh = undrawn(
        &db,
        &["Heliod, Sun-Crowned", "Walking Ballista"],
        &["Plains", "Plains"],
    );
    let told = AiSession::from_game(&fresh)
        .with_combos(
            P0,
            vec![combo(
                "damage",
                &["Heliod, Sun-Crowned", "Walking Ballista"],
            )],
        )
        .with_rehearsals(
            P0,
            [("damage".to_string(), Rehearsal::Wins { with: Vec::new() })],
        );
    assert!(!combo_work_due(&fresh, &told, P0, &config));
}

/// What P0 casts in its main phase with Walking Ballista in hand, four lands and nothing else
/// to do, the combo's other two cards still in its library. Plays P0's decisions until it
/// passes with an empty stack.
fn cast_with_ballista_in_hand(db: &CardDatabase, knows: Option<Rehearsal>) -> Vec<String> {
    let mut board = Board::new(db);
    board.hand("Walking Ballista");
    board.library("Basalt Monolith");
    board.library("Power Artifact");
    board.lands("Wastes", 4);
    let mut state = board.build();
    state.loop_detection = LoopDetectionMode::Interactive;
    let entry = combo("mana", &["Basalt Monolith", "Power Artifact"]);
    let mut session = AiSession::from_game(&state);
    if let Some(known) = knows {
        session = session
            .with_combos(P0, vec![entry.clone()])
            .with_rehearsals(P0, [(entry.id.clone(), known)]);
    }
    let session = Arc::new(session);
    let config =
        create_config_for_players(AiDifficulty::Medium, Platform::Native, 2).into_measurement(SEED);
    let ai_players: HashSet<PlayerId> = [P0].into_iter().collect();
    let ai_configs: HashMap<PlayerId, _> = [(P0, config)].into_iter().collect();
    let mut rng = SmallRng::seed_from_u64(SEED);
    let mut cast = Vec::new();
    for _ in 0..20 {
        let passing_on_empty = state.stack.is_empty()
            && matches!(state.waiting_for, WaitingFor::Priority { player } if player == P0);
        let before = state.clone();
        let ControlFlow::Continue(result) =
            run_ai_action(&mut state, &ai_players, &ai_configs, &mut rng, &session)
        else {
            break;
        };
        if let GameAction::CastSpell { object_id, .. } = &result.action {
            cast.push(object_name(&before, *object_id));
        }
        if passing_on_empty && matches!(result.action, GameAction::PassPriority) {
            break;
        }
    }
    cast
}

/// A seat putting a combo together keeps the spell that will spend what the combo makes: with
/// Basalt Monolith and Power Artifact still to find, it holds Walking Ballista instead of
/// casting it as a small creature. A seat that knows nothing of the combo, or whose rehearsal
/// showed no win, casts it.
#[test]
fn a_seat_holds_its_payoff_until_the_combo_is_in_place() {
    let db = integration_card_db();
    let wins = Rehearsal::Wins {
        with: vec!["Walking Ballista".to_string()],
    };
    assert_eq!(
        cast_with_ballista_in_hand(&db, Some(wins)),
        Vec::<String>::new()
    );
    assert_eq!(
        cast_with_ballista_in_hand(&db, None),
        vec!["Walking Ballista".to_string()]
    );
    assert_eq!(
        cast_with_ballista_in_hand(&db, Some(Rehearsal::NoWin)),
        vec!["Walking Ballista".to_string()]
    );
}

// ---------------------------------------------------------------------------------------------
// The decision contract: what a certified line may submit beyond the candidate sample.
// ---------------------------------------------------------------------------------------------

/// Basalt Monolith's two activations on the Basalt Monolith board: the mana ability and the
/// untap.
fn basalt_abilities(state: &GameState) -> (GameAction, GameAction) {
    let monolith = state
        .objects
        .iter()
        .find(|(_, object)| object.name == "Basalt Monolith")
        .map(|(id, _)| *id)
        .expect("Basalt Monolith is on the board");
    let activate = |ability_index| GameAction::ActivateAbility {
        source_id: monolith,
        ability_index,
        payment_mode: CastPaymentMode::Auto,
    };
    let mana = engine::game::mana_sources::activatable_mana_actions_for_player(state, P0)
        .into_iter()
        .find(|action| {
            matches!(action, GameAction::ActivateAbility { source_id, .. } if *source_id == monolith)
        })
        .expect("the untapped Monolith's mana ability is activatable");
    let untap = [0, 1]
        .into_iter()
        .map(activate)
        .find(|action| *action != mana)
        .expect("the other ability");
    (mana, untap)
}

/// CR 605.3a: a permanent's mana ability is in the seat's priority domain though the
/// candidate sample leaves it out, and only for the seat that holds priority.
#[test]
fn the_decision_contract_admits_a_permanents_mana_ability_at_priority() {
    let db = integration_card_db();
    let state = basalt_power_artifact_ballista(&db);
    let (mana, _) = basalt_abilities(&state);
    let contract = AiDecisionContract::issue(&state, P0);
    assert!(
        !contract
            .candidates
            .iter()
            .any(|candidate| candidate.action == mana),
        "the sample leaves the mana ability out"
    );
    assert!(contract.contains_action(&state, &mana));

    // An activation the mana enumerator does not list is still refused.
    let GameAction::ActivateAbility { source_id, .. } = mana else {
        unreachable!("a mana ability activation");
    };
    let unlisted = GameAction::ActivateAbility {
        source_id,
        ability_index: 99,
        payment_mode: CastPaymentMode::Auto,
    };
    assert!(!contract.contains_action(&state, &unlisted));
    // The seat without priority gets nothing from this.
    assert!(!AiDecisionContract::issue(&state, P1).contains_action(&state, &mana));
}

/// CR 732.2a: at an offer for an unbounded loop the sample publishes no count for the seat to
/// take, and the contract admits each count from one to the offer's capacity with the
/// engine's own declaration, and none outside it.
#[test]
fn the_decision_contract_admits_a_count_within_an_unbounded_offers_capacity() {
    let db = integration_card_db();
    let mut state = basalt_power_artifact_ballista(&db);
    state.loop_detection = LoopDetectionMode::Interactive;
    let (mana, untap) = basalt_abilities(&state);
    let apply = |state: &mut GameState, action: GameAction| {
        engine::game::engine::apply_as_current_for_simulation(state, action)
            .expect("the reducer takes the action");
    };
    // Tap for mana and untap, lap after lap, declining any offer only to repeat a step,
    // until the engine offers the loop itself.
    let turn = state.turn_number;
    let mut laps = [mana, untap].into_iter().cycle();
    let (capacity, suggestion) = loop {
        match &state.waiting_for {
            WaitingFor::LoopShortcut {
                certificate,
                schema,
                declaration,
                ..
            } if certificate.win_kind == Some(WinKind::Advantage) => {
                break (
                    schema.deliverable_capacity,
                    schema.suggestion(declaration.as_ref()).cloned(),
                );
            }
            WaitingFor::LoopShortcut { .. } => apply(&mut state, GameAction::DeclineShortcut),
            WaitingFor::Priority { player } if *player == P0 && state.stack.is_empty() => {
                let next = laps.next().expect("a cycle does not end");
                apply(&mut state, next);
            }
            WaitingFor::Priority { .. } => apply(&mut state, GameAction::PassPriority),
            other => panic!("an unexpected prompt on the way to the offer: {other:?}"),
        }
        assert_eq!(
            state.turn_number, turn,
            "the engine offered no loop for Basalt Monolith under Power Artifact"
        );
    };

    let declare = |count| GameAction::DeclareShortcut {
        count: IterationCount::Fixed(count),
        template: suggestion.clone(),
    };
    let contract = AiDecisionContract::issue(&state, P0);
    assert!(
        !contract
            .candidates
            .iter()
            .any(|candidate| matches!(candidate.action, GameAction::DeclareShortcut { .. })),
        "the sample offers this seat only the decline"
    );
    assert!(capacity > 5);
    for count in [1, 5, capacity] {
        assert!(contract.contains_action(&state, &declare(count)), "{count}");
    }
    for count in [0, capacity + 1] {
        assert!(
            !contract.contains_action(&state, &declare(count)),
            "{count}"
        );
    }
    // Only the proposer may declare.
    assert!(!AiDecisionContract::issue(&state, P1).contains_action(&state, &declare(5)));
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

// ---------------------------------------------------------------------------------------------
// Whole games: does knowing the deck's combo change how the seat does?
// ---------------------------------------------------------------------------------------------

/// The combo seat's list, blue-black: Basalt Monolith and Power Artifact (the catalogued combo,
/// one copy each), Walking Ballista (the payoff the catalog does not name, one copy), eight
/// tutors (six that find any card), seven card-draw spells, eighteen fair creatures and 24 lands.
fn knowledge_combo_deck() -> Vec<String> {
    let mut deck: Vec<String> = Vec::new();
    let mut add = |name: &str, copies: usize| {
        deck.extend(std::iter::repeat(name.to_string()).take(copies));
    };
    add("Basalt Monolith", 1);
    add("Power Artifact", 1);
    add("Walking Ballista", 1);
    add("Demonic Tutor", 4);
    add("Beseech the Mirror", 2);
    add("Fabricate", 2);
    add("Sign in Blood", 3);
    add("Divination", 2);
    add("Preordain", 2);
    add("Walking Corpse", 4);
    add("Gifted Aetherborn", 4);
    add("Barony Vampire", 4);
    add("Dross Crocodile", 3);
    add("Bane Alley Blackguard", 3);
    add("Island", 12);
    add("Swamp", 12);
    deck
}

/// The other seat: a fair red-green creature deck.
fn knowledge_fair_deck() -> Vec<String> {
    let mut deck: Vec<String> = Vec::new();
    let mut add = |name: &str, copies: usize| {
        deck.extend(std::iter::repeat(name.to_string()).take(copies));
    };
    add("Llanowar Elves", 4);
    add("Grizzly Bears", 4);
    add("Balduvian Bears", 4);
    add("Centaur Courser", 4);
    add("Alpine Grizzly", 4);
    add("Hill Giant", 4);
    add("Elvish Warrior", 4);
    add("Lightning Bolt", 4);
    add("Shivan Dragon", 2);
    add("Fire Elemental", 2);
    add("Mountain", 12);
    add("Forest", 12);
    deck
}

const KNOWLEDGE_PIECES: [&str; 3] = ["Basalt Monolith", "Power Artifact", "Walking Ballista"];

/// What the combo seat was told before the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Knowledge {
    /// Nothing: the AI as it was before catalogued combos.
    None,
    /// The catalogued combo, and that rehearsing it showed no win: it plays the combo when it
    /// happens to hold it, and does not look for it. This is the AI before rehearsals.
    ComboNotLookedFor,
    /// The catalogued combo, rehearsed in the game's first turns.
    Rehearsed,
}

#[derive(Debug, Default)]
struct WholeGame {
    winner: Option<PlayerId>,
    turn: u32,
    /// The combo seat declared a loop shortcut.
    looped: bool,
    /// The combo seat took one of the pieces with a library search.
    tutored_pieces: usize,
    /// The combo seat held all three pieces at once, somewhere it could use them.
    assembled: bool,
    rehearsals: Vec<(String, Rehearsal)>,
}

/// One whole two-player game at 20 life between the combo seat (P0) and the fair seat (P1),
/// Medium AI on both, loop detection on. Odd seeds let the fair seat go first.
fn whole_game(db: &CardDatabase, seed: u64, knowledge: Knowledge) -> WholeGame {
    use engine::game::deck_loading::{
        load_and_hydrate_decks, resolve_deck_list, DeckList, PlayerDeckList,
    };

    const TURN_CAP: u32 = 30;
    const ACTION_CAP: usize = 6000;

    let payload = resolve_deck_list(
        db,
        &DeckList {
            player: PlayerDeckList {
                main_deck: knowledge_combo_deck(),
                ..Default::default()
            },
            opponent: PlayerDeckList {
                main_deck: knowledge_fair_deck(),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    let mut state = GameState::new_two_player(seed);
    load_and_hydrate_decks(&mut state, &payload, Some(db));
    state.loop_detection = LoopDetectionMode::Interactive;
    let first = if seed % 2 == 0 { P0 } else { P1 };
    engine::game::engine::start_game_with_starting_player(&mut state, first);

    let combo = combo(
        "basalt-power-artifact",
        &["Basalt Monolith", "Power Artifact"],
    );
    let session = AiSession::from_game(&state);
    let session = match knowledge {
        Knowledge::None => session,
        Knowledge::ComboNotLookedFor => session
            .with_combos(P0, vec![combo.clone()])
            .with_rehearsals(P0, [(combo.id.clone(), Rehearsal::NoWin)]),
        Knowledge::Rehearsed => session.with_combos(P0, vec![combo.clone()]),
    };
    let session = Arc::new(session);
    let config =
        create_config_for_players(AiDifficulty::Medium, Platform::Native, 2).into_measurement(seed);
    let ai_players: HashSet<PlayerId> = [P0, P1].into_iter().collect();
    let ai_configs: HashMap<PlayerId, _> =
        [(P0, config.clone()), (P1, config)].into_iter().collect();
    let mut rng = SmallRng::seed_from_u64(seed);

    let piece = |state: &GameState, id: &ObjectId| {
        state
            .objects
            .get(id)
            .is_some_and(|object| KNOWLEDGE_PIECES.iter().any(|name| object.name == *name))
    };
    let held = |state: &GameState| {
        KNOWLEDGE_PIECES.iter().all(|name| {
            state.objects.values().any(|object| {
                object.name == *name
                    && object.owner == P0
                    && matches!(object.zone, Zone::Hand | Zone::Battlefield)
            })
        })
    };

    let trace = std::env::var("KNOWLEDGE_TRACE").is_ok_and(|value| value == seed.to_string());
    let mut game = WholeGame::default();
    let mut actions = 0;
    loop {
        if let WaitingFor::GameOver { winner } = &state.waiting_for {
            game.winner = *winner;
            break;
        }
        if state.turn_number > TURN_CAP || actions >= ACTION_CAP {
            break;
        }
        game.assembled |= held(&state);
        let Some((owner, _)) = phase_ai::auto_play::eligible_ai_decision(&state, &ai_players)
        else {
            break;
        };
        let searching = matches!(state.waiting_for, WaitingFor::SearchChoice { .. });
        let before = state.clone();
        match run_ai_action(&mut state, &ai_players, &ai_configs, &mut rng, &session) {
            ControlFlow::Continue(result) => {
                actions += 1;
                if trace && owner == P0 {
                    let touches_piece = match &result.action {
                        GameAction::CastSpell { object_id, .. } => piece(&before, object_id),
                        GameAction::ActivateAbility { source_id, .. } => piece(&before, source_id),
                        GameAction::SelectCards { cards } => {
                            cards.iter().any(|id| piece(&before, id))
                        }
                        GameAction::ChooseX { .. } | GameAction::DeclareShortcut { .. } => true,
                        _ => false,
                    };
                    let tutor = matches!(&result.action, GameAction::CastSpell { object_id, .. }
                        if before.objects.get(object_id).is_some_and(|o| ["Demonic Tutor", "Beseech the Mirror", "Fabricate"].contains(&o.name.as_str())));
                    if touches_piece || tutor || searching {
                        let zones: Vec<String> = KNOWLEDGE_PIECES
                            .iter()
                            .map(|name| {
                                let zone = before
                                    .objects
                                    .values()
                                    .find(|o| o.name == *name && o.owner == P0)
                                    .map_or("-".to_string(), |o| format!("{:?}", o.zone));
                                format!("{}={zone}", &name[..4])
                            })
                            .collect();
                        eprintln!(
                            "  t{} {:?} [{}] {}",
                            before.turn_number,
                            knowledge,
                            zones.join(" "),
                            label(&before, &result.action).unwrap_or_else(|| format!(
                                "{:?}",
                                result.action
                            )
                            .chars()
                            .take(120)
                            .collect())
                        );
                    }
                }
                if owner == P0 {
                    match &result.action {
                        GameAction::DeclareShortcut { .. } => game.looped = true,
                        GameAction::SelectCards { cards } if searching => {
                            game.tutored_pieces +=
                                cards.iter().filter(|id| piece(&before, id)).count();
                        }
                        _ => {}
                    }
                }
            }
            ControlFlow::Break(_) => break,
        }
    }
    game.turn = state.turn_number;
    game.rehearsals = session.rehearsals(P0);
    game
}

/// Whole games, the same seeds for each kind of knowledge. Prints the combo seat's wins, how
/// fast, how often it assembled the combo, how often it went off, and how many pieces its
/// tutors fetched. Set `KNOWLEDGE_GAMES` for more games (default 20) and `KNOWLEDGE_THREADS`
/// for more threads (default 4).
#[test]
#[ignore = "measurement, not a gate: plays whole games and prints a table"]
fn deck_knowledge_in_whole_games() {
    let games: u64 = std::env::var("KNOWLEDGE_GAMES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(20);
    let threads: usize = std::env::var("KNOWLEDGE_THREADS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(4);
    let db = integration_card_db();
    let arms = [
        Knowledge::None,
        Knowledge::ComboNotLookedFor,
        Knowledge::Rehearsed,
    ];
    let jobs: Vec<(Knowledge, u64)> = arms
        .iter()
        .flat_map(|arm| (0..games).map(move |seed| (*arm, 1000 + seed)))
        .collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results = std::sync::Mutex::new(Vec::<(Knowledge, u64, WholeGame, f64)>::new());
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| loop {
                let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some((arm, seed)) = jobs.get(index).copied() else {
                    break;
                };
                let started = std::time::Instant::now();
                let game = whole_game(&db, seed, arm);
                let seconds = started.elapsed().as_secs_f64();
                eprintln!(
                    "{arm:?} seed {seed}: winner {:?} turn {} looped {} assembled {} tutored {} ({seconds:.1}s)",
                    game.winner, game.turn, game.looped, game.assembled, game.tutored_pieces
                );
                results.lock().unwrap().push((arm, seed, game, seconds));
            });
        }
    });
    let results = results.into_inner().unwrap();
    println!(
        "\n{:<20} {:>6} {:>6} {:>10} {:>10} {:>8} {:>9} {:>9} {:>8}",
        "knowledge",
        "games",
        "wins",
        "win turn",
        "assembled",
        "looped",
        "tutored",
        "undecided",
        "sec/game"
    );
    for arm in arms {
        let rows: Vec<&(Knowledge, u64, WholeGame, f64)> =
            results.iter().filter(|row| row.0 == arm).collect();
        let wins: Vec<u32> = rows
            .iter()
            .filter(|row| row.2.winner == Some(P0))
            .map(|row| row.2.turn)
            .collect();
        let mean_turn = if wins.is_empty() {
            f64::NAN
        } else {
            f64::from(wins.iter().sum::<u32>()) / wins.len() as f64
        };
        println!(
            "{:<20} {:>6} {:>6} {:>10.1} {:>10} {:>8} {:>9} {:>9} {:>8.1}",
            format!("{arm:?}"),
            rows.len(),
            wins.len(),
            mean_turn,
            rows.iter().filter(|row| row.2.assembled).count(),
            rows.iter().filter(|row| row.2.looped).count(),
            rows.iter().map(|row| row.2.tutored_pieces).sum::<usize>(),
            rows.iter().filter(|row| row.2.winner.is_none()).count(),
            rows.iter().map(|row| row.3).sum::<f64>() / rows.len().max(1) as f64,
        );
    }
    // The paired view: the same seed with and without the rehearsal.
    let by_seed = |arm: Knowledge| -> HashMap<u64, bool> {
        results
            .iter()
            .filter(|row| row.0 == arm)
            .map(|row| (row.1, row.2.winner == Some(P0)))
            .collect()
    };
    let before = by_seed(Knowledge::ComboNotLookedFor);
    let after = by_seed(Knowledge::Rehearsed);
    let gained = after
        .iter()
        .filter(|(seed, won)| **won && !before[*seed])
        .count();
    let lost = after
        .iter()
        .filter(|(seed, won)| !**won && before[*seed])
        .count();
    println!("\nsame seeds, rehearsed against not looked for: {gained} games turned into wins, {lost} wins turned into losses");
    if let Some(row) = results.iter().find(|row| row.0 == Knowledge::Rehearsed) {
        println!(
            "what a rehearsal showed (seed {}): {:?}",
            row.1, row.2.rehearsals
        );
    }
}
