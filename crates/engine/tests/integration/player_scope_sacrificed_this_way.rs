//! "Each other player sacrifices a creature of their choice. You create a 2/2
//! black Zombie creature token for each creature sacrificed this way." (Syphon
//! Flesh) and "Each player sacrifices a creature of their choice. Create an X/X
//! black Demon creature token with flying, where X is the total power of the
//! creatures sacrificed this way." (Reign of the Pit)
//!
//! CR 608.2c + CR 701.21a: the instruction after a per-player sacrifice reads
//! the permanents that sacrifice moved. CR 608.2h + CR 113.7a: their power is
//! taken as they last existed on the battlefield. Before the fix the per-player
//! sacrifice never published what it sacrificed, so the count and the total
//! power were 0 (no Zombie; a 0/0 Demon that died).

use engine::game::scenario::{GameScenario, P0, P1};
use engine::types::game_state::{GameState, WaitingFor};
use engine::types::mana::ManaCost;
use engine::types::phase::Phase;
use engine::types::zones::Zone;

const SYPHON_FLESH: &str = "Each other player sacrifices a creature of their choice. You create a 2/2 black Zombie creature token for each creature sacrificed this way.";
const REIGN_OF_THE_PIT: &str = "Each player sacrifices a creature of their choice. Create an X/X black Demon creature token with flying, where X is the total power of the creatures sacrificed this way.";

fn tokens_named<'a>(
    state: &'a GameState,
    name: &'a str,
) -> Vec<&'a engine::game::game_object::GameObject> {
    state
        .battlefield
        .iter()
        .filter_map(|id| state.objects.get(id))
        .filter(|obj| obj.controller == P0 && obj.is_token && obj.name == name)
        .collect()
}

/// The opponent's only creature is sacrificed without a choice.
#[test]
fn one_zombie_per_creature_sacrificed_without_a_choice() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let mine = scenario.add_creature(P0, "Grizzly Bears", 2, 2).id();
    let theirs = scenario.add_creature(P1, "Grizzly Bears", 2, 2).id();
    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, "Syphon Flesh", false, SYPHON_FLESH)
        .with_mana_cost(ManaCost::generic(0))
        .id();

    let mut runner = scenario.build();
    let outcome = runner.cast(spell).resolve();

    assert_eq!(outcome.zone_of(theirs), Zone::Graveyard);
    assert_eq!(
        outcome.zone_of(mine),
        Zone::Battlefield,
        "each OTHER player"
    );
    assert_eq!(tokens_named(outcome.state(), "Zombie").len(), 1);
}

/// The opponent chooses between two creatures; one is sacrificed, one Zombie.
#[test]
fn one_zombie_per_creature_sacrificed_after_a_choice() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let sacrificed = scenario.add_creature(P1, "Grizzly Bears", 2, 2).id();
    let survivor = scenario.add_creature(P1, "Serra Angel", 4, 4).id();
    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, "Syphon Flesh", false, SYPHON_FLESH)
        .with_mana_cost(ManaCost::generic(0))
        .id();

    let mut runner = scenario.build();
    // CR 608.2d: declare the opponent's sacrifice selection; an unanswered
    // EffectZoneChoice is a paused spell, not a completed zero-token result.
    let outcome = runner.cast(spell).effect_zone(&[sacrificed]).resolve();

    assert!(matches!(
        outcome.final_waiting_for(),
        WaitingFor::Priority { .. }
    ));
    assert!(outcome.state().stack.is_empty());
    assert_eq!(outcome.zone_of(sacrificed), Zone::Graveyard);
    assert_eq!(outcome.zone_of(survivor), Zone::Battlefield);
    assert_eq!(tokens_named(outcome.state(), "Zombie").len(), 1);
}

/// Two 2/2s sacrificed: the Demon is 4/4 and survives.
#[test]
fn demon_is_the_total_power_of_the_creatures_sacrificed() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario.add_creature(P0, "Grizzly Bears", 2, 2);
    scenario.add_creature(P1, "Grizzly Bears", 2, 2);
    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, "Reign of the Pit", false, REIGN_OF_THE_PIT)
        .with_mana_cost(ManaCost::generic(0))
        .id();

    let mut runner = scenario.build();
    let outcome = runner.cast(spell).resolve();

    let demons = tokens_named(outcome.state(), "Demon");
    assert_eq!(demons.len(), 1, "the Demon survives as a 4/4");
    assert_eq!((demons[0].power, demons[0].toughness), (Some(4), Some(4)));
}
