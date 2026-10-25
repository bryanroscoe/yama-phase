//! Sepulchral Primordial — "When this creature enters, for each opponent, you
//! may put up to one target creature card from that player's graveyard onto
//! the battlefield under your control."
//!
//! CR 115.1 + CR 601.2c + CR 108.4a: up to one target from each opponent's
//! graveyard is any number of targets from opponents' graveyards, no two
//! controlled (owned) by the same player. Before the fix the trigger took one
//! target in all.

use engine::game::scenario::{GameScenario, P0, P1};
use engine::types::mana::{ManaColor, ManaCost};
use engine::types::phase::Phase;
use engine::types::player::PlayerId;
use engine::types::zones::Zone;

const P2: PlayerId = PlayerId(2);

/// Verbatim Oracle text (MTGJSON), keyword line included.
const SEPULCHRAL_PRIMORDIAL: &str = "Intimidate (This creature can't be blocked except by artifact creatures and/or creatures that share a color with it.)\nWhen this creature enters, for each opponent, you may put up to one target creature card from that player's graveyard onto the battlefield under your control.";

#[test]
fn sepulchral_primordial_takes_a_creature_card_from_each_opponents_graveyard() {
    let mut scenario = GameScenario::new_n_player(3, 7);
    scenario.at_phase(Phase::PreCombatMain);
    for _ in 0..7 {
        scenario.add_basic_land(P0, ManaColor::Black);
    }
    let primordial = scenario
        .add_creature_to_hand_from_oracle(P0, "Sepulchral Primordial", 5, 4, SEPULCHRAL_PRIMORDIAL)
        .with_mana_cost(ManaCost::generic(7))
        .id();
    let giant = scenario.add_creature_to_graveyard(P1, "Hill Giant", 3, 3).id();
    let wurm = scenario.add_creature_to_graveyard(P2, "Craw Wurm", 6, 4).id();
    let mut runner = scenario.build();

    let outcome = runner
        .cast(primordial)
        .target_objects(&[giant, wurm])
        .accept_optional()
        .resolve();

    for id in [giant, wurm] {
        let card = &outcome.state().objects[&id];
        assert_eq!(card.zone, Zone::Battlefield, "{} came back", card.name);
        assert_eq!(card.controller, P0, "under the Primordial's controller");
    }
}
