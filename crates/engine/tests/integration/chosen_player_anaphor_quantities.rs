//! "Choose an opponent. If that player has more cards in hand than you, draw
//! cards equal to the difference." (Sandstone Oracle's enters trigger.)
//!
//! CR 608.2c + CR 109.4: "that player" is the opponent chosen this way. Before
//! the fix the hand-size anaphor read the controller, so the comparison was
//! never true and nothing was drawn. CR 121.1: the difference is determined
//! once, then that many cards are drawn.

use engine::game::scenario::{GameScenario, P0, P1};
use engine::types::game_state::WaitingFor;
use engine::types::mana::ManaCost;
use engine::types::phase::Phase;

const DRAW_THE_DIFFERENCE: &str =
    "Choose an opponent. If that player has more cards in hand than you, draw cards equal to the difference.";

#[test]
fn draws_the_difference_when_the_chosen_opponent_has_more_cards() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario.with_cards_in_hand(P0, &["Wastes"]);
    scenario.with_cards_in_hand(P1, &["Wastes", "Wastes", "Wastes"]);
    scenario.with_library_top(P0, &["Wastes", "Wastes", "Wastes"]);
    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, "Oracle's Count", false, DRAW_THE_DIFFERENCE)
        .with_mana_cost(ManaCost::generic(0))
        .id();

    let mut runner = scenario.build();
    // CR 608.2d: answer the non-targeted opponent choice; the driver otherwise
    // stops at NamedChoice before the comparison and draw can run.
    let outcome = runner.cast(spell).choose_option("1").resolve();

    assert!(matches!(
        outcome.final_waiting_for(),
        WaitingFor::Priority { .. }
    ));
    assert!(outcome.state().stack.is_empty());

    // One card left in hand against three: draw two.
    outcome.assert_hand_drawn(P0, 2);
}

#[test]
fn draws_nothing_when_the_chosen_opponent_has_no_more_cards() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario.with_cards_in_hand(P0, &["Wastes", "Wastes"]);
    scenario.with_cards_in_hand(P1, &["Wastes"]);
    scenario.with_library_top(P0, &["Wastes", "Wastes"]);
    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, "Oracle's Count", false, DRAW_THE_DIFFERENCE)
        .with_mana_cost(ManaCost::generic(0))
        .id();

    let mut runner = scenario.build();
    // CR 608.2d: answer the non-targeted opponent choice; the driver otherwise
    // stops at NamedChoice before the comparison and draw can run.
    let outcome = runner.cast(spell).choose_option("1").resolve();

    assert!(matches!(
        outcome.final_waiting_for(),
        WaitingFor::Priority { .. }
    ));
    assert!(outcome.state().stack.is_empty());
    outcome.assert_hand_drawn(P0, 0);
}
