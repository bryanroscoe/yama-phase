//! Unbreakable Formation: "Creatures you control gain indestructible until end
//! of turn. Addendum — If you cast this spell during your main phase, put a
//! +1/+1 counter on each of those creatures and they gain vigilance until end
//! of turn."
//!
//! CR 608.2c: "those creatures" names the creatures the first sentence affected,
//! and "they" names the same creatures again. The counter instruction consumes
//! that population and relays it to the vigilance grant; it is not a competing
//! producer, so the first sentence must still publish it. Before the fix the
//! head declined to publish (a later node sat in publisher position), so the
//! addendum put no counters and granted nothing.
//!
//! CR 505.1 + CR 207.2c: the addendum checks that the spell was cast during its
//! controller's main phase.

use engine::game::scenario::{GameScenario, P0, P1};
use engine::types::counter::CounterType;
use engine::types::keywords::Keyword;
use engine::types::mana::ManaCost;
use engine::types::phase::Phase;

const UNBREAKABLE_FORMATION: &str =
    "Creatures you control gain indestructible until end of turn.\n\
    Addendum — If you cast this spell during your main phase, put a +1/+1 counter on each of those \
    creatures and they gain vigilance until end of turn.";

fn plus_one_counters(
    state: &engine::types::game_state::GameState,
    id: engine::types::identifiers::ObjectId,
) -> u32 {
    state.objects[&id]
        .counters
        .get(&CounterType::Plus1Plus1)
        .copied()
        .unwrap_or(0)
}

#[test]
fn addendum_in_main_phase_counters_and_vigilance_on_those_creatures() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let mine = scenario.add_creature(P0, "Grizzly Bears", 2, 2).id();
    let theirs = scenario.add_creature(P1, "Grizzly Bears", 2, 2).id();
    let formation = scenario
        .add_spell_to_hand_from_oracle(P0, "Unbreakable Formation", true, UNBREAKABLE_FORMATION)
        .with_mana_cost(ManaCost::generic(0))
        .id();

    let mut runner = scenario.build();
    let outcome = runner.cast(formation).resolve();
    let state = outcome.state();

    assert_eq!(plus_one_counters(state, mine), 1);
    assert!(state.objects[&mine].has_keyword(&Keyword::Indestructible));
    assert!(state.objects[&mine].has_keyword(&Keyword::Vigilance));
    assert_eq!(outcome.power_toughness(mine), (3, 3));

    // Opponents' creatures are never "creatures you control".
    assert_eq!(plus_one_counters(state, theirs), 0);
    assert!(!state.objects[&theirs].has_keyword(&Keyword::Indestructible));
    assert!(!state.objects[&theirs].has_keyword(&Keyword::Vigilance));
}

#[test]
fn addendum_outside_main_phase_gives_indestructible_only() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::End);
    let mine = scenario.add_creature(P0, "Grizzly Bears", 2, 2).id();
    let formation = scenario
        .add_spell_to_hand_from_oracle(P0, "Unbreakable Formation", true, UNBREAKABLE_FORMATION)
        .with_mana_cost(ManaCost::generic(0))
        .id();

    let mut runner = scenario.build();
    let outcome = runner.cast(formation).resolve();
    let state = outcome.state();

    assert!(state.objects[&mine].has_keyword(&Keyword::Indestructible));
    assert_eq!(plus_one_counters(state, mine), 0);
    assert!(!state.objects[&mine].has_keyword(&Keyword::Vigilance));
}
