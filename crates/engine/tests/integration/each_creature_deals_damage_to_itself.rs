//! Wave of Reckoning / Solar Blaze: "Each creature deals damage to itself equal
//! to its power."
//!
//! CR 120.1 + CR 120.3e: each creature is the source of damage dealt to itself,
//! in the amount of its own power as the effect is applied; the damage is
//! marked on it and state-based actions destroy those with lethal damage
//! (CR 704.5g).

use engine::game::scenario::{GameScenario, P0, P1};
use engine::types::mana::ManaCost;
use engine::types::phase::Phase;
use engine::types::zones::Zone;

const WAVE_OF_RECKONING: &str = "Each creature deals damage to itself equal to its power.";

#[test]
fn each_creature_deals_damage_equal_to_its_power_to_itself() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let bears = scenario.add_creature(P0, "Grizzly Bears", 2, 2).id();
    let their_bears = scenario.add_creature(P1, "Grizzly Bears", 2, 2).id();
    let angel = scenario.add_creature(P1, "Serra Angel", 4, 4).id();
    let wall = scenario.add_creature(P1, "Wall of Stone", 0, 4).id();
    let elder = scenario.add_creature(P0, "Sage Elder", 1, 3).id();
    let wave = scenario
        .add_spell_to_hand_from_oracle(P0, "Wave of Reckoning", false, WAVE_OF_RECKONING)
        .with_mana_cost(ManaCost::generic(0))
        .id();

    let mut runner = scenario.build();
    let outcome = runner.cast(wave).resolve();
    let state = outcome.state();

    for dead in [bears, their_bears, angel] {
        assert_eq!(
            outcome.zone_of(dead),
            Zone::Graveyard,
            "power >= toughness: lethal damage from itself"
        );
    }
    assert_eq!(outcome.zone_of(wall), Zone::Battlefield);
    assert_eq!(
        state.objects[&wall].damage_marked, 0,
        "0 power deals no damage"
    );
    assert_eq!(outcome.zone_of(elder), Zone::Battlefield);
    assert_eq!(state.objects[&elder].damage_marked, 1, "1/3 marks 1 damage");
}
