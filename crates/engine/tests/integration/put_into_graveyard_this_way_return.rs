//! "Destroy all creatures, then return a creature card put into a graveyard this
//! way to the battlefield under your control." (Necromantic Selection)
//!
//! CR 608.2c + CR 400.7j: the return chooses among the cards the destruction put
//! into graveyards, never a card that was already there.

use engine::game::scenario::{GameScenario, P0, P1};
use engine::types::game_state::WaitingFor;
use engine::types::phase::Phase;
use engine::types::zones::Zone;

const NECROMANTIC_SELECTION: &str = "Destroy all creatures, then return a creature card put into a graveyard this way to the battlefield under your control. It's a black Zombie in addition to its other colors and types. Exile Necromantic Selection.";

#[test]
fn return_offers_only_the_creature_cards_destroyed_this_way() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let oak = scenario.add_creature(P0, "Oak", 2, 3).id();
    let elm = scenario.add_creature(P1, "Elm", 1, 7).id();
    let earlier = scenario.add_creature_to_graveyard(P0, "Earlier", 9, 9).id();
    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, "Necromantic Selection", false, NECROMANTIC_SELECTION)
        .id();
    let mut runner = scenario.build();

    let outcome = runner.cast(spell).resolve();

    outcome.assert_zone(&[oak, elm, earlier], Zone::Graveyard);
    match outcome.final_waiting_for() {
        WaitingFor::EffectZoneChoice {
            player,
            cards,
            count,
            destination,
            enters_under_player,
            ..
        } => {
            assert_eq!(*player, P0);
            let mut offered = cards.clone();
            offered.sort();
            let mut destroyed = vec![oak, elm];
            destroyed.sort();
            assert_eq!(
                offered, destroyed,
                "only the creatures destroyed this way are offered, not {earlier:?}"
            );
            assert_eq!(*count, 1);
            assert_eq!(*destination, Some(Zone::Battlefield));
            assert_eq!(*enters_under_player, Some(P0));
        }
        other => panic!("expected the return choice, got {other:?}"),
    }
}

#[test]
fn chosen_card_returns_under_the_casters_control() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let oak = scenario.add_creature(P0, "Oak", 2, 3).id();
    let elm = scenario.add_creature(P1, "Elm", 1, 7).id();
    let earlier = scenario.add_creature_to_graveyard(P0, "Earlier", 9, 9).id();
    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, "Necromantic Selection", false, NECROMANTIC_SELECTION)
        .id();
    let mut runner = scenario.build();

    let outcome = runner.cast(spell).effect_zone(&[elm]).resolve();

    outcome.assert_zone(&[elm], Zone::Battlefield);
    outcome.assert_zone(&[oak, earlier], Zone::Graveyard);
    let returned = outcome.state().objects.get(&elm).expect("Elm exists");
    assert_eq!(
        returned.controller, P0,
        "Elm returns under its caster's control"
    );
}
