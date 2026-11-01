//! CR 117.3b and 800.4j: priority after the active player leaves during resolution.
use super::support::shared_card_db;
use engine::ai_support::AiDecisionContract;
use engine::game::engine::{apply_as_current, apply_verified_ai_priority_pass};
use engine::game::scenario::{GameScenario, P0};
use engine::game::scenario_db::GameScenarioDbExt;
use engine::types::actions::GameAction;
use engine::types::game_state::WaitingFor;
use engine::types::mana::ManaColor;
use engine::types::phase::Phase;
use engine::types::player::PlayerId;
use engine::types::zones::Zone;

#[test]
fn active_player_departure_never_gets_a_trigger_priority_window() {
    let db = shared_card_db().unwrap();
    let p2 = PlayerId(2);
    let mut s = GameScenario::new_n_player(3, 51);
    s.at_phase(Phase::PreCombatMain).with_life(P0, 1);
    let valgavoth = s.add_real_card(p2, "Valgavoth, Harrower of Souls", Zone::Battlefield, db);
    s.with_library_top(p2, &["Plains", "Island", "Swamp"]);
    let shock = s.add_real_card(P0, "Shock", Zone::Hand, db);
    s.add_basic_land(P0, ManaColor::Red);
    let mut r = s.build();
    r.cast(shock).target_player(P0).commit();
    let mut saw_departure = false;
    for _ in 0..20 {
        let WaitingFor::Priority { player } = r.state().waiting_for else {
            panic!("unexpected prompt {:?}", r.state().waiting_for)
        };
        // CR 800.4j: the turn continues, but priority goes to the next living player.
        assert!(
            !r.state().players[player.0 as usize].is_eliminated,
            "departed priority actor {player:?}"
        );
        saw_departure |= r.state().players[0].is_eliminated;
        if r.state().stack.is_empty() {
            break;
        }
        let contract = AiDecisionContract::issue(r.state(), player);
        apply_verified_ai_priority_pass(r.state_mut(), player, &contract, GameAction::PassPriority)
            .unwrap();
    }
    assert!(saw_departure);
    assert!(r.state().stack.is_empty());
    assert_eq!(
        r.state().active_player,
        P0,
        "departure does not start another turn"
    );
    assert_eq!(
        r.state().players[2].hand.len(),
        1,
        "Valgavoth resolved exactly once"
    );
    assert!(r.state().objects.contains_key(&valgavoth));
    apply_as_current(r.state_mut(), GameAction::PassPriority).unwrap();
}

fn departing_representative_keeps_the_correct_priority(
    format: engine::types::format::FormatConfig,
    direction: engine::types::phase::TurnDirection,
    expected: PlayerId,
) {
    let db = shared_card_db().unwrap();
    let p1 = PlayerId(1);
    let mut s = GameScenario::new_with_format(format, 3, 51);
    s.at_phase(Phase::PreCombatMain).with_life(p1, 1);
    s.add_real_card(P0, "Valgavoth, Harrower of Souls", Zone::Battlefield, db);
    s.with_library_top(P0, &["Plains", "Island", "Swamp"]);
    let shock = s.add_real_card(p1, "Shock", Zone::Hand, db);
    s.add_basic_land(p1, ManaColor::Red);
    let mut r = s.build();
    r.state_mut().active_player = p1;
    r.state_mut().priority_player = p1;
    r.state_mut().waiting_for = WaitingFor::Priority { player: p1 };
    r.state_mut().turn_direction = direction;
    r.cast(shock).target_player(p1).commit();
    assert!(
        !r.state().stack.is_empty(),
        "the lethal spell must be on the stack"
    );
    let mut saw_trigger_after_departure = false;
    for _ in 0..20 {
        if r.state().stack.is_empty() {
            break;
        }
        saw_trigger_after_departure |= r.state().players[1].is_eliminated;
        assert!(matches!(r.state().waiting_for, WaitingFor::Priority { .. }));
        apply_as_current(r.state_mut(), GameAction::PassPriority).unwrap();
    }
    assert!(
        r.state().players[1].is_eliminated,
        "the active representative left during resolution"
    );
    assert!(
        saw_trigger_after_departure,
        "a trigger must resolve after the representative leaves"
    );
    assert_eq!(
        r.state().players[0].hand.len(),
        1,
        "the trigger resolved exactly once"
    );
    assert_eq!(r.state().turn_direction, direction);
    assert!(!r.state().players[0].is_eliminated);
    assert!(!r.state().players[2].is_eliminated);
    assert!(
        r.state().stack.is_empty(),
        "the lethal spell finished resolving"
    );
    assert_eq!(
        r.state().active_player,
        p1,
        "departure preserves the current turn"
    );
    assert!(
        matches!(r.state().waiting_for, WaitingFor::Priority { player } if player == expected),
        "priority must belong to {expected:?}, got {:?}",
        r.state().waiting_for
    );
    assert_eq!(r.state().priority_player, expected);
    apply_as_current(r.state_mut(), GameAction::PassPriority).unwrap();
}

#[test]
fn active_team_survivor_receives_priority_in_both_turn_directions() {
    use engine::types::{format::FormatConfig, phase::TurnDirection};
    for direction in [TurnDirection::Normal, TurnDirection::Reversed] {
        departing_representative_keeps_the_correct_priority(
            FormatConfig::archenemy(),
            direction,
            PlayerId(2),
        );
    }
}

#[test]
fn departed_free_for_all_player_uses_next_seat_in_turn_direction() {
    use engine::types::{format::FormatConfig, phase::TurnDirection};
    for (direction, expected) in [
        (TurnDirection::Normal, PlayerId(2)),
        (TurnDirection::Reversed, P0),
    ] {
        departing_representative_keeps_the_correct_priority(
            FormatConfig::standard(),
            direction,
            expected,
        );
    }
}

// CR 800.4a + 800.4j: removing the last attacker while a defender's choice
// is open makes that choice empty. The public action boundary finishes the
// empty declaration and must publish the surviving priority recipient.
fn blocker_completion_after_concession(departing: Option<PlayerId>, expected: PlayerId) {
    use engine::game::combat::AttackTarget;
    use engine::game::scenario::P1;

    let mut s = GameScenario::new_n_player(3, 51);
    s.at_phase(Phase::BeginCombat);
    let attacker = s.add_creature(P0, "Attacker", 2, 2).id();
    s.add_creature(P1, "Potential blocker", 1, 3);
    let mut r = s.build();
    r.declare_attackers(&[(attacker, AttackTarget::Player(P1))])
        .unwrap();
    for _ in 0..3 {
        assert!(matches!(r.state().waiting_for, WaitingFor::Priority { .. }));
        apply_as_current(r.state_mut(), GameAction::PassPriority).unwrap();
    }
    assert!(
        matches!(
            r.state().waiting_for,
            WaitingFor::DeclareBlockers { player: P1, .. }
        ),
        "expected the blocker choice, got {:?}",
        r.state().waiting_for
    );
    let turn = r.state().turn_number;
    if let Some(player_id) = departing {
        r.act(GameAction::Concede { player_id }).unwrap();
        assert!(r.state().players[player_id.0 as usize].is_eliminated);
    }
    if departing == Some(P0) {
        assert_ne!(r.state().objects[&attacker].zone, Zone::Battlefield);
        assert!(!engine::game::combat::has_attackers_in_play(r.state()));
        // The real concession action already completed the now-empty blocker
        // declaration; no test-side prompt repair or extra action is needed.
    } else {
        assert!(matches!(
            r.state().waiting_for,
            WaitingFor::DeclareBlockers { player: P1, .. }
        ));
        r.declare_blockers(&[]).unwrap();
    }
    assert_eq!(r.state().active_player, P0);
    assert_eq!(r.state().turn_number, turn);
    assert_eq!(r.state().phase, Phase::DeclareBlockers);
    assert!(
        matches!(
            r.state().waiting_for,
            WaitingFor::Priority { player } if player == expected
        ),
        "expected priority for {expected:?}, got {:?}",
        r.state().waiting_for
    );
    assert_eq!(r.state().priority_player, expected);
    apply_as_current(r.state_mut(), GameAction::PassPriority).unwrap();
    let WaitingFor::Priority { player } = r.state().waiting_for else {
        panic!(
            "expected the next survivor's window: {:?}",
            r.state().waiting_for
        );
    };
    assert_ne!(player, expected);
    assert!(!r.state().players[player.0 as usize].is_eliminated);
}

#[test]
fn blocker_completion_after_active_concession_grants_survivor_priority() {
    blocker_completion_after_concession(Some(P0), PlayerId(1));
}

#[test]
fn blocker_completion_after_nonactive_concession_keeps_active_priority() {
    blocker_completion_after_concession(Some(PlayerId(2)), P0);
}

#[test]
fn blocker_completion_without_concession_keeps_active_priority() {
    blocker_completion_after_concession(None, P0);
}
