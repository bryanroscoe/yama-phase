//! Zaffai, Thunder Conductor — "Magecraft — Whenever you cast or copy an
//! instant or sorcery spell, scry 1. If that spell's mana value is 5 or
//! greater, create a 4/4 blue and red Elemental creature token. If that
//! spell's mana value is 10 or greater, Zaffai deals 10 damage to an opponent
//! chosen at random."
//!
//! CR 603.3d + CR 608.2c: the trigger's instructions resolve in order. Before
//! the fix, once the scry was answered, the rest of the chain (the token node,
//! whose later damage node holds the opponent) was put back on the stack as a
//! new triggered ability asking for that target, and resolving it did the same
//! again, for ever: the Elemental was never made and games hit the action cap.

use engine::ai_support::AiDecisionContract;
use engine::game::scenario::{GameRunner, GameScenario, P0};
use engine::types::actions::GameAction;
use engine::types::game_state::{CastPaymentMode, WaitingFor};
use engine::types::mana::{ManaColor, ManaCost};
use engine::types::phase::Phase;
use engine::types::zones::Zone;

/// Verbatim Oracle text (MTGJSON).
const ZAFFAI: &str = "Magecraft \u{2014} Whenever you cast or copy an instant or sorcery spell, scry 1. If that spell's mana value is 5 or greater, create a 4/4 blue and red Elemental creature token. If that spell's mana value is 10 or greater, Zaffai deals 10 damage to an opponent chosen at random.";

/// Answer every decision with the engine's first offer until the stack is
/// empty; `None` if it never settles.
fn settle(runner: &mut GameRunner) -> Option<usize> {
    for step in 0..80 {
        if matches!(runner.state().waiting_for, WaitingFor::Priority { .. })
            && runner.state().stack.is_empty()
        {
            return Some(step);
        }
        let player = runner.state().waiting_for.acting_players()[0];
        let action = AiDecisionContract::issue(runner.state(), player)
            .candidates
            .into_iter()
            .map(|candidate| candidate.action)
            .find(|action| !matches!(action, GameAction::CancelCast))
            .expect("the engine offers an answer");
        runner
            .act(action.clone())
            .unwrap_or_else(|e| panic!("{action:?} refused: {e}"));
    }
    None
}

#[test]
fn zaffai_makes_the_elemental_for_a_five_mana_sorcery_and_settles() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario.add_creature_from_oracle(P0, "Zaffai, Thunder Conductor", 1, 4, ZAFFAI);
    for _ in 0..5 {
        scenario.add_basic_land(P0, ManaColor::Blue);
    }
    for name in ["Island", "Island", "Island"] {
        scenario.add_card_to_library_top(P0, name);
    }
    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, "Big Sorcery", false, "Draw a card.")
        .with_mana_cost(ManaCost::generic(5))
        .id();
    let mut runner = scenario.build();
    let card_id = runner.state().objects[&spell].card_id;
    runner
        .act(GameAction::CastSpell {
            object_id: spell,
            card_id,
            targets: vec![],
            payment_mode: CastPaymentMode::Auto,
        })
        .expect("cast the sorcery");

    assert!(
        settle(&mut runner).is_some(),
        "the magecraft trigger and the spell resolve and the game settles"
    );
    let elementals = runner
        .state()
        .battlefield
        .iter()
        .filter(|id| {
            let o = &runner.state().objects[*id];
            o.is_token && o.card_types.subtypes.iter().any(|s| s == "Elemental")
        })
        .count();
    assert_eq!(elementals, 1, "one 4/4 Elemental for a five-mana spell");
    assert_eq!(runner.state().objects[&spell].zone, Zone::Graveyard);
}
