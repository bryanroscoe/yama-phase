//! Furygale Flocking — "This spell costs {1} less to cast for each instant
//! and sorcery card in your graveyard. For each opponent, create two 3/3 blue
//! and red Elemental creature tokens with flying that attack that opponent
//! this turn if able. They gain haste until end of turn."
//!
//! Two seams, each failing before the fix:
//! - the token keyword list stops at the relative clause "that attack that
//!   opponent …", so the tokens have flying;
//! - CR 608.2c: "They" names every token the repeated instruction made, so
//!   all four get haste, not only the last opponent's two.
//!
//! Not covered (not implemented): the requirement to attack that opponent.

use engine::game::scenario::{GameScenario, P0};
use engine::types::keywords::Keyword;
use engine::types::mana::{ManaColor, ManaCost};
use engine::types::phase::Phase;

/// Verbatim Oracle text (MTGJSON).
const FURYGALE_FLOCKING: &str = "This spell costs {1} less to cast for each instant and sorcery card in your graveyard.\nFor each opponent, create two 3/3 blue and red Elemental creature tokens with flying that attack that opponent this turn if able. They gain haste until end of turn.";

#[test]
fn furygale_flocking_makes_two_hasty_flyers_for_each_opponent() {
    let mut scenario = GameScenario::new_n_player(3, 7);
    scenario.at_phase(Phase::PreCombatMain);
    for _ in 0..10 {
        scenario.add_basic_land(P0, ManaColor::Red);
    }
    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, "Furygale Flocking", false, FURYGALE_FLOCKING)
        .with_mana_cost(ManaCost::generic(10))
        .id();
    let mut runner = scenario.build();
    let outcome = runner.cast(spell).resolve();

    let tokens: Vec<_> = outcome
        .state()
        .battlefield
        .iter()
        .filter(|id| outcome.state().objects[*id].is_token)
        .copied()
        .collect();
    assert_eq!(tokens.len(), 4, "two for each of two opponents");
    for id in tokens {
        let token = &outcome.state().objects[&id];
        assert!(token.keywords.contains(&Keyword::Flying), "flying");
        assert!(
            token.keywords.contains(&Keyword::Haste),
            "haste for every token"
        );
        assert_eq!((token.power, token.toughness), (Some(3), Some(3)));
    }
}
