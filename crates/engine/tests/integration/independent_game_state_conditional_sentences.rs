//! Sentences gated on their own game-state checks are separate instructions.
//!
//! Sunset Revelry: "If an opponent has more life than you, you gain 4 life. /
//! If an opponent controls more creatures than you, create two 1/1 white Human
//! creature tokens. / If an opponent has more cards in hand than you, draw a
//! card."
//!
//! CR 608.2c: the instructions are followed in the order written, and each
//! "If ..." sentence is checked when it is reached. A false check on the first
//! sentence (equal life totals) says nothing about the second and third, whose
//! conditions read only the game state and nothing the first sentence did.
//! Before the fix the false first gate suppressed every later conditional
//! sentence, so the spell did nothing.

use engine::game::scenario::{GameScenario, P0, P1};
use engine::types::mana::ManaCost;
use engine::types::phase::Phase;
use engine::types::zones::Zone;

const SUNSET_REVELRY: &str = "If an opponent has more life than you, you gain 4 life.\n\
    If an opponent controls more creatures than you, create two 1/1 white Human creature tokens.\n\
    If an opponent has more cards in hand than you, draw a card.";

fn human_tokens(state: &engine::types::game_state::GameState) -> usize {
    state
        .battlefield
        .iter()
        .filter_map(|id| state.objects.get(id))
        .filter(|obj| obj.controller == P0 && obj.is_token && obj.name == "Human")
        .count()
}

/// Equal life (first sentence false), more opposing creatures and cards in
/// hand (second and third true): two Humans and a card, no life.
#[test]
fn later_sentences_resolve_after_a_false_first_sentence() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario.add_creature(P0, "Grizzly Bears", 2, 2);
    scenario.add_creature(P1, "Grizzly Bears", 2, 2);
    scenario.add_creature(P1, "Serra Angel", 4, 4);
    scenario.with_cards_in_hand(P0, &["Wastes"]);
    scenario.with_cards_in_hand(P1, &["Wastes", "Wastes"]);
    scenario.with_library_top(P0, &["Wastes", "Wastes"]);
    let revelry = scenario
        .add_spell_to_hand_from_oracle(P0, "Sunset Revelry", false, SUNSET_REVELRY)
        .with_mana_cost(ManaCost::generic(0))
        .id();

    let mut runner = scenario.build();
    let outcome = runner.cast(revelry).resolve();

    assert_eq!(outcome.life_delta(P0), 0, "equal life totals: no life gain");
    assert_eq!(
        human_tokens(outcome.state()),
        2,
        "an opponent controls more creatures: two Human tokens"
    );
    outcome.assert_hand_drawn(P0, 1);
    assert_eq!(outcome.zone_of(revelry), Zone::Graveyard);
}

/// Every check true: all three sentences happen.
#[test]
fn all_sentences_resolve_when_every_check_holds() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario.with_life(P1, 25);
    scenario.add_creature(P1, "Grizzly Bears", 2, 2);
    scenario.with_cards_in_hand(P1, &["Wastes"]);
    scenario.with_library_top(P0, &["Wastes"]);
    let revelry = scenario
        .add_spell_to_hand_from_oracle(P0, "Sunset Revelry", false, SUNSET_REVELRY)
        .with_mana_cost(ManaCost::generic(0))
        .id();

    let mut runner = scenario.build();
    let outcome = runner.cast(revelry).resolve();

    assert_eq!(outcome.life_delta(P0), 4);
    assert_eq!(human_tokens(outcome.state()), 2);
    outcome.assert_hand_drawn(P0, 1);
}

/// Every check false (ties): nothing happens.
#[test]
fn ties_satisfy_no_sentence() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario.add_creature(P0, "Grizzly Bears", 2, 2);
    scenario.add_creature(P1, "Grizzly Bears", 2, 2);
    scenario.with_cards_in_hand(P0, &["Wastes"]);
    scenario.with_cards_in_hand(P1, &["Wastes"]);
    scenario.with_library_top(P0, &["Wastes"]);
    let revelry = scenario
        .add_spell_to_hand_from_oracle(P0, "Sunset Revelry", false, SUNSET_REVELRY)
        .with_mana_cost(ManaCost::generic(0))
        .id();

    let mut runner = scenario.build();
    let outcome = runner.cast(revelry).resolve();

    assert_eq!(outcome.life_delta(P0), 0);
    assert_eq!(human_tokens(outcome.state()), 0);
    outcome.assert_hand_drawn(P0, 0);
}
