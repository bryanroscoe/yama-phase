//! CR 107.14 + CR 122.1 + CR 602.1b + CR 602.5: "Activate only if you've paid
//! or lost four or more {E} this turn" (Izzet Generatorium) restricts when the
//! ability may be activated. Paying {E} removes energy counters, so energy paid
//! for a cost and energy lost to an effect count alike, per player and per turn;
//! energy the player still has doesn't count.

use engine::ai_support::legal_actions;
use engine::game::casting::can_activate_ability_now;
use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::types::ability::{
    AbilityCost, ActivationRestriction, Comparator, ParsedCondition, QuantityExpr, QuantityRef,
};
use engine::types::actions::GameAction;
use engine::types::game_state::CastPaymentMode;
use engine::types::identifiers::ObjectId;
use engine::types::phase::Phase;
use engine::types::player::PlayerId;
use engine::types::resolved_commands::ResolvedPlayerEdit;

/// Izzet Generatorium (Oracle text verified against Scryfall).
const IZZET_GENERATORIUM: &str = "If you would get one or more {E} (energy counters), you get that many plus one {E} instead.\n{T}: Draw a card. Activate only if you've paid or lost four or more {E} this turn.";

/// Synthetic (labelled): a repeatable way to pay {E}.
const ENERGY_BATTERY: &str = "Pay {E}: You gain 1 life.";

struct Board {
    runner: GameRunner,
    generatorium: ObjectId,
    battery: ObjectId,
}

fn board(energy: u32) -> Board {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario.with_library_top(P0, &["Wastes"; 8]);
    scenario.with_library_top(P1, &["Wastes"; 8]);
    let generatorium = scenario
        .add_artifact_from_oracle(P0, "Izzet Generatorium", IZZET_GENERATORIUM)
        .id();
    let battery = scenario
        .add_artifact_from_oracle(P0, "Energy Battery", ENERGY_BATTERY)
        .id();
    let mut runner = scenario.build();
    runner.state_mut().players[0].energy = energy;
    Board {
        runner,
        generatorium,
        battery,
    }
}

fn tap_ability_index(runner: &GameRunner, source: ObjectId) -> usize {
    runner.state().objects[&source]
        .abilities
        .iter()
        .position(|ability| matches!(ability.cost, Some(AbilityCost::Tap)))
        .expect("the {T} ability")
}

fn offers_activation(runner: &GameRunner, source: ObjectId, index: usize) -> bool {
    legal_actions(runner.state()).iter().any(|action| {
        matches!(
            action,
            GameAction::ActivateAbility { source_id, ability_index, .. }
                if *source_id == source && *ability_index == index
        )
    })
}

/// Pay {E} through the battery `times` times, resolving each activation.
fn pay_energy(runner: &mut GameRunner, battery: ObjectId, times: u32) {
    for _ in 0..times {
        runner
            .act(GameAction::ActivateAbility {
                source_id: battery,
                ability_index: 0,
                payment_mode: CastPaymentMode::Auto,
            })
            .expect("Pay {E} is accepted");
        runner.advance_until_stack_empty();
    }
}

fn lose_energy(runner: &mut GameRunner, player: PlayerId, amount: i32) {
    runner
        .state_mut()
        .resolve_and_apply_player_edit(player, ResolvedPlayerEdit::Energy { delta: -amount })
        .expect("the player has the energy");
}

#[test]
fn the_restriction_parses_as_energy_paid_or_lost_this_turn() {
    let b = board(0);
    let index = tap_ability_index(&b.runner, b.generatorium);
    let restrictions =
        &b.runner.state().objects[&b.generatorium].abilities[index].activation_restrictions;
    assert_eq!(
        restrictions,
        &vec![ActivationRestriction::RequiresCondition {
            condition: Some(ParsedCondition::QuantityComparison {
                lhs: QuantityExpr::Ref {
                    qty: QuantityRef::EnergyPaidOrLostThisTurn,
                },
                comparator: Comparator::GE,
                rhs: QuantityExpr::Fixed { value: 4 },
            }),
        }]
    );
}

#[test]
fn not_activatable_when_no_energy_was_paid_this_turn() {
    for energy in [3, 10] {
        let mut b = board(energy);
        let index = tap_ability_index(&b.runner, b.generatorium);
        assert!(
            !can_activate_ability_now(b.runner.state(), P0, b.generatorium, index),
            "having {energy} energy isn't paying it"
        );
        assert!(!offers_activation(&b.runner, b.generatorium, index));
        assert!(
            b.runner
                .act(GameAction::ActivateAbility {
                    source_id: b.generatorium,
                    ability_index: index,
                    payment_mode: CastPaymentMode::Auto,
                })
                .is_err(),
            "a submitted activation is refused"
        );
        assert!(!b.runner.state().objects[&b.generatorium].tapped);
    }
}

#[test]
fn paying_four_energy_makes_it_activatable_for_the_rest_of_the_turn() {
    let mut b = board(5);
    let index = tap_ability_index(&b.runner, b.generatorium);
    pay_energy(&mut b.runner, b.battery, 3);
    assert_eq!(b.runner.state().players[0].energy_paid_or_lost_this_turn, 3);
    assert!(
        !offers_activation(&b.runner, b.generatorium, index),
        "three paid isn't four"
    );
    pay_energy(&mut b.runner, b.battery, 1);
    assert_eq!(b.runner.state().players[0].energy, 1);
    assert_eq!(b.runner.state().players[0].energy_paid_or_lost_this_turn, 4);
    assert!(offers_activation(&b.runner, b.generatorium, index));
    // Gaining energy again doesn't undo what was paid.
    b.runner.state_mut().players[0].energy = 9;
    assert!(offers_activation(&b.runner, b.generatorium, index));

    let hand_before = b.runner.state().players[0].hand.len();
    b.runner
        .act(GameAction::ActivateAbility {
            source_id: b.generatorium,
            ability_index: index,
            payment_mode: CastPaymentMode::Auto,
        })
        .expect("the activation is legal");
    b.runner.advance_until_stack_empty();
    assert_eq!(b.runner.state().players[0].hand.len(), hand_before + 1);
}

#[test]
fn paid_and_lost_energy_are_summed() {
    let mut b = board(5);
    let index = tap_ability_index(&b.runner, b.generatorium);
    pay_energy(&mut b.runner, b.battery, 2);
    lose_energy(&mut b.runner, P0, 2);
    assert_eq!(b.runner.state().players[0].energy_paid_or_lost_this_turn, 4);
    assert!(offers_activation(&b.runner, b.generatorium, index));
}

#[test]
fn an_opponents_energy_does_not_count() {
    let mut b = board(0);
    let index = tap_ability_index(&b.runner, b.generatorium);
    b.runner.state_mut().players[1].energy = 6;
    lose_energy(&mut b.runner, P1, 6);
    assert_eq!(b.runner.state().players[1].energy_paid_or_lost_this_turn, 6);
    assert!(!can_activate_ability_now(
        b.runner.state(),
        P0,
        b.generatorium,
        index
    ));
}

#[test]
fn the_count_resets_at_the_start_of_each_turn() {
    let mut b = board(5);
    let index = tap_ability_index(&b.runner, b.generatorium);
    pay_energy(&mut b.runner, b.battery, 4);
    assert!(can_activate_ability_now(
        b.runner.state(),
        P0,
        b.generatorium,
        index
    ));
    b.runner.advance_to_upkeep();
    assert_ne!(b.runner.state().active_player, P0, "the next turn began");
    assert_eq!(b.runner.state().players[0].energy_paid_or_lost_this_turn, 0);
    assert!(
        !can_activate_ability_now(b.runner.state(), P0, b.generatorium, index),
        "energy paid on an earlier turn doesn't count"
    );
}
