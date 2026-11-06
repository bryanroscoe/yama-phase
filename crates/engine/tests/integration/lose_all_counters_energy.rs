//! CR 122.1 + CR 107.14: energy counters are counters, so a player who "loses
//! all counters" loses their energy too, and that energy counts as lost this
//! turn ("you've paid or lost four or more {E} this turn").

use engine::game::casting::can_activate_ability_now;
use engine::game::scenario::{GameScenario, P0, P1};
use engine::types::ability::AbilityCost;
use engine::types::identifiers::ObjectId;
use engine::types::mana::{ManaCost, ManaCostShard, ManaType, ManaUnit};
use engine::types::phase::Phase;

/// Final Act (Oracle text verified against Scryfall).
const FINAL_ACT: &str = "Choose one or more —\n• Destroy all creatures.\n• Destroy all planeswalkers.\n• Destroy all battles.\n• Exile all graveyards.\n• Each opponent loses all counters.";

/// Izzet Generatorium (Oracle text verified against Scryfall).
const IZZET_GENERATORIUM: &str = "If you would get one or more {E} (energy counters), you get that many plus one {E} instead.\n{T}: Draw a card. Activate only if you've paid or lost four or more {E} this turn.";

#[test]
fn losing_all_counters_loses_energy_and_counts_it_as_lost() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario.with_library_top(P1, &["Wastes"; 4]);
    let generatorium = scenario
        .add_artifact_from_oracle(P1, "Izzet Generatorium", IZZET_GENERATORIUM)
        .id();
    let final_act = scenario
        .add_spell_to_hand_from_oracle(P0, "Final Act", false, FINAL_ACT)
        .with_mana_cost(ManaCost::Cost {
            shards: vec![ManaCostShard::Black, ManaCostShard::Black],
            generic: 4,
        })
        .id();
    scenario.with_mana_pool(
        P0,
        (0..6)
            .map(|_| ManaUnit::new(ManaType::Black, ObjectId(0), false, vec![]))
            .collect(),
    );
    let mut runner = scenario.build();
    runner.state_mut().players[1].energy = 4;
    let index = runner.state().objects[&generatorium]
        .abilities
        .iter()
        .position(|ability| matches!(ability.cost, Some(AbilityCost::Tap)))
        .expect("the {T} ability");
    assert!(!can_activate_ability_now(
        runner.state(),
        P1,
        generatorium,
        index
    ));

    runner.cast(final_act).modes(&[4]).resolve();

    assert_eq!(
        runner.state().players[1].energy,
        0,
        "energy counters are counters"
    );
    assert_eq!(runner.state().players[1].energy_paid_or_lost_this_turn, 4);
    assert!(
        can_activate_ability_now(runner.state(), P1, generatorium, index),
        "four {{E}} lost this turn enables Izzet Generatorium"
    );
}
