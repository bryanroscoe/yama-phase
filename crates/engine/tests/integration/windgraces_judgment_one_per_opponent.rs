//! Windgrace's Judgment — "For any number of opponents, destroy target
//! nonland permanent that player controls."
//!
//! CR 115.1 + CR 601.2c: choosing any number of opponents and, for each, one
//! target that opponent controls is choosing any number of targets that
//! opponents control, no two controlled by the same player. Before the fix
//! the sentence parsed as an unimplemented clause head plus a destroy of a
//! permanent YOU control, so the spell could not be cast at all.

use engine::game::scenario::{GameScenario, P0, P1};
use engine::types::mana::{ManaColor, ManaCost};
use engine::types::phase::Phase;
use engine::types::player::PlayerId;
use engine::types::zones::Zone;

const P2: PlayerId = PlayerId(2);

/// Verbatim Oracle text (MTGJSON).
const WINDGRACES_JUDGMENT: &str =
    "For any number of opponents, destroy target nonland permanent that player controls.";

#[test]
fn windgraces_judgment_destroys_one_permanent_of_each_chosen_opponent() {
    let mut scenario = GameScenario::new_n_player(3, 7);
    scenario.at_phase(Phase::PreCombatMain);
    for _ in 0..5 {
        scenario.add_basic_land(P0, ManaColor::Black);
    }
    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, "Windgrace's Judgment", true, WINDGRACES_JUDGMENT)
        .with_mana_cost(ManaCost::generic(5))
        .id();
    let bears = scenario.add_creature(P1, "Grizzly Bears", 2, 2).id();
    let giant = scenario.add_creature(P2, "Hill Giant", 3, 3).id();
    let mut runner = scenario.build();

    let outcome = runner.cast(spell).target_objects(&[bears, giant]).resolve();

    assert_eq!(outcome.state().objects[&bears].zone, Zone::Graveyard);
    assert_eq!(outcome.state().objects[&giant].zone, Zone::Graveyard);
}
