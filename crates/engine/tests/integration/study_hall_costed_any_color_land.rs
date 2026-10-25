//! CR 605.3b + CR 601.2g + CR 117.1d: a land's "{1}, {T}: Add one mana of any
//! color" (Study Hall) can be activated while paying, so a spell whose color
//! only that ability makes, with generic mana besides, is castable.
//!
//! Study Hall, a Mountain and three Wastes pay for {2}{W}: one Wastes pays
//! Study Hall's {1}, Study Hall makes {W}, and the Mountain and a Wastes pay
//! {2}. The mana-source list offers Study Hall's ability as one selection whose
//! color is chosen as it resolves, but tapping a land checked that selection
//! against the per-color rows only, so the activation was refused as stale.
//! The castability check walks exactly that route, so the spell was never
//! offered.

use engine::ai_support::legal_actions_full;
use engine::game::mana_sources::activatable_mana_source_selections;
use engine::game::scenario::{GameScenario, P0};
use engine::types::actions::GameAction;
use engine::types::game_state::{ManaChoice, WaitingFor};
use engine::types::mana::{ManaColor, ManaCost, ManaCostShard, ManaSourceOutput, ManaType};
use engine::types::phase::Phase;

/// Verbatim Oracle text (MTGJSON).
const STUDY_HALL: &str = "{T}: Add {C}.\n{1}, {T}: Add one mana of any color. When you spend this mana to cast your commander, scry X, where X is the number of times it's been cast from the command zone this game.";

#[test]
fn study_hall_makes_the_color_a_spell_with_generic_mana_needs() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let hall = scenario.add_land_from_oracle(P0, "Study Hall", STUDY_HALL).id();
    scenario.add_basic_land(P0, ManaColor::Red);
    let wastes = scenario
        .add_land_from_oracle(P0, "Wastes", "{T}: Add {C}.")
        .id();
    scenario.add_land_from_oracle(P0, "Wastes", "{T}: Add {C}.");
    scenario.add_land_from_oracle(P0, "Wastes", "{T}: Add {C}.");
    let spell = scenario
        .add_creature_to_hand(P0, "White Three-Drop", 2, 2)
        .with_mana_cost(ManaCost::Cost {
            shards: vec![ManaCostShard::White],
            generic: 2,
        })
        .id();
    let mut runner = scenario.build();

    let (actions, _, _) = legal_actions_full(runner.state());
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, GameAction::CastSpell { object_id, .. } if *object_id == spell)),
        "{{2}}{{W}} is payable with {{W}} from Study Hall"
    );

    // The route by hand: a Wastes for the {1}, then Study Hall's any-color
    // ability, whose color is asked for as it resolves.
    let selections = activatable_mana_source_selections(runner.state(), P0);
    let wastes_tap = selections
        .iter()
        .find(|s| s.source.object_id == wastes)
        .expect("the Wastes taps for mana")
        .clone();
    runner
        .act(GameAction::TapLandForMana {
            selection: wastes_tap,
        })
        .expect("tap the Wastes");
    let hall_any_color = activatable_mana_source_selections(runner.state(), P0)
        .into_iter()
        .find(|s| s.source.object_id == hall && s.ability_index == Some(1))
        .expect("Study Hall's {1}, {T} ability is offered with {C} floating");
    assert!(matches!(
        hall_any_color.output,
        ManaSourceOutput::DeferredColorChoice { .. }
    ));
    runner
        .act(GameAction::TapLandForMana {
            selection: hall_any_color,
        })
        .expect("the offered selection activates Study Hall");
    assert!(
        matches!(runner.state().waiting_for, WaitingFor::ChooseManaColor { .. }),
        "the color is chosen as the ability resolves: {:?}",
        runner.state().waiting_for
    );
    runner
        .act(GameAction::ChooseManaColor {
            choice: ManaChoice::SingleColor(ManaType::White),
            count: 1,
        })
        .expect("choose white");
    let pool = &runner.state().players[0].mana_pool;
    assert_eq!(pool.count_color(ManaType::White), 1);
    assert_eq!(pool.count_color(ManaType::Colorless), 0, "the {{1}} was paid");
    assert!(runner.state().objects[&hall].tapped);
}
