//! CR 205.3i + CR 604.3 — Planar Nexus: "This land is every nonbasic land type."
//!
//! The line is a characteristic-defining ability (printed on the land, defines
//! only its own subtypes, unconditional), so once the layers are applied the
//! land has every nonbasic land type and none of the five basic land types
//! (CR 305.6). Before the fix the sentence parsed as an unimplemented spell
//! effect and the land had no subtypes at all, so Desert, Gate, Locus and
//! Urza's payoffs never counted it.

use engine::game::layers::evaluate_layers;
use engine::game::scenario::{GameScenario, P0};
use engine::types::phase::Phase;

/// Verbatim Oracle text (MTGJSON), reminder text included.
const PLANAR_NEXUS: &str = "This land is every nonbasic land type. (Nonbasic land types include Cave, Desert, Gate, Lair, Locus, Mine, Power-Plant, Sphere, Tower, and Urza's.)\n{T}: Add {C}.\n{1}, {T}: Add one mana of any color.";

#[test]
fn planar_nexus_is_every_nonbasic_land_type_and_no_basic_one() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let nexus = scenario
        .add_land_from_oracle(P0, "Planar Nexus", PLANAR_NEXUS)
        .id();
    let mut runner = scenario.build();
    evaluate_layers(runner.state_mut());

    let subtypes = &runner.state().objects[&nexus].card_types.subtypes;
    for nonbasic in [
        "Cave",
        "Desert",
        "Gate",
        "Lair",
        "Locus",
        "Mine",
        "Planet",
        "Power-Plant",
        "Sphere",
        "Tower",
        "Town",
        "Urza's",
    ] {
        assert!(
            subtypes.iter().any(|s| s == nonbasic),
            "Planar Nexus must be a {nonbasic}: {subtypes:?}"
        );
    }
    for basic in ["Plains", "Island", "Swamp", "Mountain", "Forest"] {
        assert!(
            !subtypes.iter().any(|s| s == basic),
            "Planar Nexus must not be a {basic} (CR 305.6 basic types): {subtypes:?}"
        );
    }
}
