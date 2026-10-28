//! CR 106.1 + CR 605.3b + CR 601.2g: an activation that adds N mana of any one
//! color adds all N at once, so a spell that needs that much mana is castable.
//!
//! Resonating Lute gives each land "{T}: Add two mana of any one color. Spend
//! this mana only to cast instant and sorcery spells." Two Islands then pay for
//! Divination ({2}{U}): one Island makes {U}{U}, the other {U}{U}. Before the
//! fix the auto-tap planner counted one mana per land for the granted ability,
//! so Divination was never offered (and a person who pays by tapping lands
//! while paying could not cast it at all). The spend restriction still holds:
//! Hill Giant ({3}{R}) can't use the Lute's mana.

use engine::ai_support::legal_actions_full;
use engine::game::scenario::{GameScenario, P0};
use engine::types::actions::GameAction;
use engine::types::mana::{ManaColor, ManaCost, ManaCostShard};
use engine::types::phase::Phase;

/// Verbatim Oracle text (MTGJSON).
const RESONATING_LUTE: &str = "Lands you control have \"{T}: Add two mana of any one color. Spend this mana only to cast instant and sorcery spells.\"\n{T}: Draw a card. Activate only if you have seven or more cards in your hand.";

fn castable(
    state: &engine::types::game_state::GameState,
    id: engine::types::identifiers::ObjectId,
) -> bool {
    let (actions, _, _) = legal_actions_full(state);
    actions
        .iter()
        .any(|a| matches!(a, GameAction::CastSpell { object_id, .. } if *object_id == id))
}

#[test]
fn two_lute_lands_pay_for_a_three_mana_sorcery_but_not_a_creature() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario
        .add_artifact_from_oracle(P0, "Resonating Lute", RESONATING_LUTE)
        .id();
    scenario.add_basic_land(P0, ManaColor::Blue);
    scenario.add_basic_land(P0, ManaColor::Blue);
    let divination = scenario
        .add_spell_to_hand_from_oracle(P0, "Divination", false, "Draw two cards.")
        .with_mana_cost(ManaCost::Cost {
            shards: vec![ManaCostShard::Blue],
            generic: 2,
        })
        .id();
    let giant = scenario
        .add_creature_to_hand(P0, "Hill Giant", 3, 3)
        .with_mana_cost(ManaCost::Cost {
            shards: vec![ManaCostShard::Red],
            generic: 3,
        })
        .id();
    let runner = scenario.build();

    assert!(
        castable(runner.state(), divination),
        "two lands with the Lute's two-mana ability make four mana for a sorcery"
    );
    assert!(
        !castable(runner.state(), giant),
        "CR 106.6: the Lute's mana can't be spent on a creature spell"
    );
}
