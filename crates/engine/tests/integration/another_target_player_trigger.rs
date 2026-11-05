//! CR 115.1d + CR 115.2 + CR 603.3d: "choose another target player" in a
//! trigger whose event names a player (The Lord of Pain) targets a player other
//! than the one the event names — the caster, per the card's ruling — never a
//! creature, and never simply "a player other than you". Other "another target
//! player" wordings (Splinter's "you and another target player") aren't claimed.

use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::parser::parse_oracle_text;
use engine::types::ability::{PlayerFilter, TargetFilter, TargetRef};
use engine::types::actions::GameAction;
use engine::types::game_state::{CastPaymentMode, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::mana::{ManaCost, ManaType, ManaUnit};
use engine::types::phase::Phase;
use engine::types::player::PlayerId;

const P2: PlayerId = PlayerId(2);

/// The Lord of Pain (Oracle text verified against Scryfall).
const THE_LORD_OF_PAIN: &str = "Menace\nYour opponents can't gain life.\nWhenever a player casts their first spell each turn, choose another target player. The Lord of Pain deals damage equal to that spell's mana value to the chosen player.";

/// Splinter, Aging Champion (Oracle text verified against Scryfall).
const SPLINTER: &str = "When Splinter enters, destroy up to one target tapped creature.\nWhen Splinter leaves the battlefield, you and another target player each draw a card.";

struct Board {
    runner: GameRunner,
    creatures: Vec<ObjectId>,
}

/// `players` seats; seat 0 controls The Lord of Pain, every other seat a
/// Grizzly Bears. `caster` is made the active player and casts a {4} sorcery
/// as its first spell of the turn.
fn cast_first_spell(players: u8, caster: PlayerId) -> Board {
    let mut scenario = GameScenario::new_n_player(players, 7);
    scenario.at_phase(Phase::PreCombatMain);
    let lord = scenario
        .add_creature_from_oracle(P0, "The Lord of Pain", 5, 5, THE_LORD_OF_PAIN)
        .id();
    let mut creatures = vec![lord];
    for seat in 1..players {
        creatures.push(scenario.add_vanilla(PlayerId(seat), 2, 2));
    }
    let spell = scenario
        .add_spell_to_hand(caster, "Four Mana Sorcery", false)
        .with_mana_cost(ManaCost::generic(4))
        .id();
    let mut runner = scenario.build();
    {
        let state = runner.state_mut();
        state.active_player = caster;
        state.priority_player = caster;
        state.waiting_for = WaitingFor::Priority { player: caster };
        let pool = &mut state.players[caster.0 as usize].mana_pool;
        for _ in 0..4 {
            pool.add(ManaUnit::new(
                ManaType::Colorless,
                ObjectId(0),
                false,
                vec![],
            ));
        }
    }
    let card_id = runner.state().objects[&spell].card_id;
    runner
        .act(GameAction::CastSpell {
            object_id: spell,
            card_id,
            targets: vec![],
            payment_mode: CastPaymentMode::Auto,
        })
        .expect("the sorcery is cast");
    Board { runner, creatures }
}

fn trigger_legal_targets(runner: &GameRunner) -> Vec<TargetRef> {
    match &runner.state().waiting_for {
        WaitingFor::TriggerTargetSelection {
            target_slots,
            selection,
            ..
        } => {
            let mut targets = target_slots[selection.current_slot].legal_targets.clone();
            targets.sort_by_key(|t| format!("{t:?}"));
            targets
        }
        other => panic!("expected the trigger's target choice, got {other:?}"),
    }
}

/// Choose `target` if the engine asks, then resolve everything.
fn choose_and_resolve(runner: &mut GameRunner, target: PlayerId) {
    if matches!(
        runner.state().waiting_for,
        WaitingFor::TriggerTargetSelection { .. }
    ) {
        runner
            .act(GameAction::ChooseTarget {
                target: Some(TargetRef::Player(target)),
            })
            .expect("a legal target");
    }
    runner.advance_until_stack_empty();
}

fn lives(runner: &GameRunner) -> Vec<i32> {
    runner.state().players.iter().map(|p| p.life).collect()
}

fn no_creature_damaged(board: &Board) {
    for creature in &board.creatures {
        assert_eq!(board.runner.state().objects[creature].damage_marked, 0);
    }
}

#[test]
fn controller_casting_excludes_the_controller() {
    let mut board = cast_first_spell(3, P0);
    assert_eq!(
        trigger_legal_targets(&board.runner),
        vec![TargetRef::Player(P1), TargetRef::Player(P2)],
        "only the players other than the caster; no creature"
    );
    choose_and_resolve(&mut board.runner, P2);
    assert_eq!(lives(&board.runner), vec![20, 20, 16]);
    no_creature_damaged(&board);
}

#[test]
fn an_opponent_casting_excludes_the_caster_not_the_controller() {
    let mut board = cast_first_spell(3, P1);
    assert_eq!(
        trigger_legal_targets(&board.runner),
        vec![TargetRef::Player(P0), TargetRef::Player(P2)],
        "the caster (seat 1) is excluded; the Lord's controller may be chosen"
    );
    choose_and_resolve(&mut board.runner, P2);
    assert_eq!(lives(&board.runner), vec![20, 20, 16]);
    no_creature_damaged(&board);

    let mut board = cast_first_spell(3, P1);
    choose_and_resolve(&mut board.runner, P0);
    assert_eq!(lives(&board.runner), vec![16, 20, 20]);
    no_creature_damaged(&board);
}

#[test]
fn two_player_controller_casting_targets_the_opponent() {
    let mut board = cast_first_spell(2, P0);
    if matches!(
        board.runner.state().waiting_for,
        WaitingFor::TriggerTargetSelection { .. }
    ) {
        assert_eq!(
            trigger_legal_targets(&board.runner),
            vec![TargetRef::Player(P1)]
        );
    }
    choose_and_resolve(&mut board.runner, P1);
    assert_eq!(lives(&board.runner), vec![20, 16]);
}

#[test]
fn two_player_opponent_casting_makes_the_controller_target_themselves() {
    // Ruling 2024-09-20: "if it's just you and one other player in the game,
    // you'll have to target yourself when that player casts their first spell".
    let mut board = cast_first_spell(2, P1);
    if matches!(
        board.runner.state().waiting_for,
        WaitingFor::TriggerTargetSelection { .. }
    ) {
        assert_eq!(
            trigger_legal_targets(&board.runner),
            vec![TargetRef::Player(P0)]
        );
    }
    choose_and_resolve(&mut board.runner, P0);
    assert_eq!(lives(&board.runner), vec![16, 20]);
    no_creature_damaged(&board);
}

#[test]
fn another_target_player_after_you_and_is_not_the_triggering_player() {
    // "you and another target player each draw a card": "another" is relative
    // to "you", not to a player the leaves-the-battlefield event names. The
    // trigger grammar's triggering-player reading must not claim it.
    let parsed = parse_oracle_text(
        SPLINTER,
        "Splinter, Aging Champion",
        &[],
        &["Creature".to_string()],
        &[],
    );
    let json = serde_json::to_string(&parsed.triggers).unwrap();
    let triggering = serde_json::to_string(&TargetFilter::PlayerMatching {
        player: Box::new(PlayerFilter::AllExcept {
            exclude: Box::new(PlayerFilter::TriggeringPlayer),
        }),
    })
    .unwrap();
    assert!(!json.contains(&triggering), "{json}");
}
