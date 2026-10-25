//! Jace, Multiverse Architect — "At the beginning of combat on each
//! opponent's turn, they may pay {2}. If they don't, creatures they control
//! can't attack Jaces you control this turn."
//!
//! Three seams, each with a failing assertion before the fix:
//! - the trigger's "on each opponent's turn" constraint (CR 603.1): it never
//!   fires on Jace's controller's own turn;
//! - the "Jaces you control" defended scope (CR 205.3j + CR 508.1c): the
//!   declining opponent's creatures can't attack Jace, but can still attack
//!   Jace's controller;
//! - the live population (CR 611.2c): the restriction is a rules change, not
//!   an ability grant, so a creature that comes under that opponent's control
//!   after the trigger resolved can't attack Jace either.

use engine::game::combat::AttackTarget;
use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::types::actions::GameAction;
use engine::types::game_state::WaitingFor;
use engine::types::identifiers::ObjectId;
use engine::types::phase::Phase;

/// Verbatim Oracle text (MTGJSON).
const JACE: &str = "At the beginning of combat on each opponent's turn, they may pay {2}. If they don't, creatures they control can't attack Jaces you control this turn.\n[+1]: Draw two cards, then put a card from your hand on the bottom of your library.\n[\u{2212}3]: Exile another target planeswalker or creature you control. Reveal cards from the top of your library until you reveal a creature or planeswalker card. Put that card onto the battlefield and the rest on the bottom of your library in a random order.\nJace, Multiverse Architect can be your commander.";

/// P0 controls Jace; P1 controls a creature and is the active player in its
/// precombat main phase.
fn setup() -> (GameRunner, ObjectId, ObjectId) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let jace = scenario
        .add_planeswalker_from_oracle(P0, "Jace, Multiverse Architect", "Jace", 4, JACE)
        .id();
    let bears = scenario.add_creature(P1, "Grizzly Bears", 2, 2).id();
    let mut runner = scenario.build();
    let state = runner.state_mut();
    state.active_player = P1;
    state.priority_player = P1;
    state.waiting_for = WaitingFor::Priority { player: P1 };
    (runner, jace, bears)
}

/// Pass to the declare-attackers step, declining Jace's {2} if it is offered.
/// Returns whether the payment was offered at all.
fn to_declare_attackers_declining(runner: &mut GameRunner) -> bool {
    let mut offered = false;
    for _ in 0..40 {
        match runner.state().waiting_for.clone() {
            WaitingFor::DeclareAttackers { .. } => return offered,
            WaitingFor::OptionalEffectChoice { .. } => {
                offered = true;
                runner
                    .act(GameAction::DecideOptionalEffect { accept: false })
                    .expect("decline Jace's payment");
            }
            WaitingFor::UnlessPayment { .. } => {
                offered = true;
                runner
                    .act(GameAction::PayUnlessCost { pay: false })
                    .expect("decline Jace's payment");
            }
            WaitingFor::Priority { .. } => {
                runner.act(GameAction::PassPriority).expect("pass priority");
            }
            other => panic!("unexpected decision on the way to combat: {other:?}"),
        }
    }
    panic!("never reached the declare-attackers step");
}

#[test]
fn an_opponent_who_does_not_pay_cannot_attack_jace() {
    let (mut runner, jace, bears) = setup();
    assert!(
        to_declare_attackers_declining(&mut runner),
        "Jace must offer P1 the {{2}} at the beginning of P1's combat"
    );
    assert!(
        runner
            .declare_attackers(&[(bears, AttackTarget::Planeswalker(jace))])
            .is_err(),
        "CR 508.1c: the Bears can't attack a Jace P0 controls this turn"
    );
    runner
        .declare_attackers(&[(bears, AttackTarget::Player(P0))])
        .expect("the restriction is scoped to Jaces: attacking P0 stays legal");
}

#[test]
fn a_creature_that_arrives_after_the_trigger_cannot_attack_jace_either() {
    let (mut runner, jace, _bears) = setup();
    assert!(to_declare_attackers_declining(&mut runner));
    // CR 611.2c: a creature P1 gains control of after the trigger resolved is
    // part of "creatures they control" too.
    let late = {
        let id = engine::game::zones::create_object(
            runner.state_mut(),
            engine::types::identifiers::CardId(9_001),
            P1,
            "Hill Giant".to_string(),
            engine::types::zones::Zone::Battlefield,
        );
        let obj = runner.state_mut().objects.get_mut(&id).unwrap();
        obj.card_types
            .core_types
            .push(engine::types::card_type::CoreType::Creature);
        obj.base_card_types = obj.card_types.clone();
        obj.power = Some(3);
        obj.toughness = Some(3);
        obj.base_power = Some(3);
        obj.base_toughness = Some(3);
        obj.summoning_sick = false;
        runner.state_mut().layers_dirty.mark_full();
        id
    };
    engine::game::layers::evaluate_layers(runner.state_mut());
    assert!(
        runner
            .declare_attackers(&[(late, AttackTarget::Planeswalker(jace))])
            .is_err(),
        "CR 611.2c: the restriction covers creatures P1 controls later"
    );
}

#[test]
fn jace_does_not_trigger_on_his_controllers_own_turn() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario.add_planeswalker_from_oracle(P0, "Jace, Multiverse Architect", "Jace", 4, JACE);
    scenario.add_creature(P0, "Grizzly Bears", 2, 2);
    let mut runner = scenario.build();
    assert!(
        !to_declare_attackers_declining(&mut runner),
        "CR 603.1: 'on each opponent's turn' — no payment is asked on P0's own turn"
    );
}
