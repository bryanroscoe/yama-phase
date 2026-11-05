//! CR 107.3 + CR 601.2b + CR 601.2f + CR 602.2b: a spell or activated ability
//! whose X carries a stated minimum ("X can't be 0") is offered only when its
//! cheapest LEGAL total — the one with X at that minimum — can be paid. X = 0
//! is not a legal announcement there, so pricing the offer at X = 0 offers an
//! action the X announcement then cancels.

use engine::ai_support::legal_actions;
use engine::game::scenario::{GameScenario, P0};
use engine::types::ability::AbilityCost;
use engine::types::actions::GameAction;
use engine::types::game_state::{CastPaymentMode, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::mana::{ManaColor, ManaCost, ManaCostShard};
use engine::types::phase::Phase;
use engine::types::zones::Zone;

/// Lair of the Hydra (Oracle text verified against Scryfall).
const LAIR_OF_THE_HYDRA: &str = "If you control two or more other lands, this land enters tapped.\n{T}: Add {G}.\n{X}{G}: Until end of turn, this land becomes an X/X green Hydra creature. It's still a land. X can't be 0.";

/// Synthetic (labelled): Lair of the Hydra's animation without the minimum, to
/// pin that an X ability with no stated minimum is still offered at X = 0.
const LAIR_WITHOUT_MINIMUM: &str = "{T}: Add {G}.\n{X}{G}: Until end of turn, this land becomes an X/X green Hydra creature. It's still a land.";

/// Synthetic (labelled) spell with a printed X minimum: "{X}{U} sorcery".
const DRAW_X_MINIMUM: &str = "Draw X cards.\nX can't be 0.";

fn x_ability_index(runner: &engine::game::scenario::GameRunner, source: ObjectId) -> usize {
    runner.state().objects[&source]
        .abilities
        .iter()
        .position(|ability| {
            matches!(
                &ability.cost,
                Some(AbilityCost::Mana { cost: ManaCost::Cost { shards, .. } })
                    if shards.contains(&ManaCostShard::X)
            )
        })
        .expect("the animation ability has {X} in its cost")
}

fn offers_activation(
    runner: &engine::game::scenario::GameRunner,
    source: ObjectId,
    index: usize,
) -> bool {
    legal_actions(runner.state()).iter().any(|action| {
        matches!(
            action,
            GameAction::ActivateAbility { source_id, ability_index, .. }
                if *source_id == source && *ability_index == index
        )
    })
}

/// Lair of the Hydra (tapped) plus `forests` untapped Forests.
fn lair_board(
    oracle: &str,
    lair_tapped: bool,
    forests: usize,
) -> (engine::game::scenario::GameRunner, ObjectId) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let lair = scenario
        .add_land_from_oracle(P0, "Lair of the Hydra", oracle)
        .id();
    for _ in 0..forests {
        scenario.add_basic_land(P0, ManaColor::Green);
    }
    let mut runner = scenario.build();
    runner.state_mut().objects.get_mut(&lair).unwrap().tapped = lair_tapped;
    (runner, lair)
}

#[test]
fn x_minimum_activation_not_offered_when_only_x_zero_is_affordable() {
    let (runner, lair) = lair_board(LAIR_OF_THE_HYDRA, true, 1);
    let index = x_ability_index(&runner, lair);
    assert_eq!(
        runner.state().objects[&lair].abilities[index].min_x_value,
        1,
        "the annotation is parsed as a minimum of 1"
    );
    assert!(
        !offers_activation(&runner, lair, index),
        "one Forest can't pay {{1}}{{G}}, the cheapest legal total"
    );
}

#[test]
fn x_minimum_activation_not_offered_from_the_source_alone() {
    // Lair can tap itself for {G}, but the ability has no {T}: one mana in all.
    let (runner, lair) = lair_board(LAIR_OF_THE_HYDRA, false, 0);
    let index = x_ability_index(&runner, lair);
    assert!(!offers_activation(&runner, lair, index));
}

#[test]
fn x_minimum_activation_offered_and_resolves_at_the_minimum() {
    let (mut runner, lair) = lair_board(LAIR_OF_THE_HYDRA, true, 2);
    let index = x_ability_index(&runner, lair);
    assert!(
        offers_activation(&runner, lair, index),
        "two Forests pay {{1}}{{G}}"
    );
    runner
        .act(GameAction::ActivateAbility {
            source_id: lair,
            ability_index: index,
            payment_mode: CastPaymentMode::Auto,
        })
        .expect("activation is accepted");
    match &runner.state().waiting_for {
        WaitingFor::ChooseXValue { min, max, .. } => {
            assert_eq!((*min, *max), (1, 1), "X = 1 is the only affordable legal X");
        }
        other => panic!("expected the X announcement, got {other:?}"),
    }
    runner
        .act(GameAction::ChooseX { value: 1 })
        .expect("X = 1 is legal");
    runner.advance_until_stack_empty();
    let lair_obj = &runner.state().objects[&lair];
    assert_eq!((lair_obj.power, lair_obj.toughness), (Some(1), Some(1)));
}

#[test]
fn x_activation_without_minimum_still_offered_at_x_zero() {
    let (runner, lair) = lair_board(LAIR_WITHOUT_MINIMUM, true, 1);
    let index = x_ability_index(&runner, lair);
    assert_eq!(
        runner.state().objects[&lair].abilities[index].min_x_value,
        0
    );
    assert!(
        offers_activation(&runner, lair, index),
        "X = 0 is a legal announcement when no minimum is stated"
    );
}

fn draw_x_board(
    islands: usize,
    extra: impl FnOnce(&mut GameScenario),
) -> (engine::game::scenario::GameRunner, ObjectId) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, "Minimum Draw", false, DRAW_X_MINIMUM)
        .with_mana_cost(ManaCost::Cost {
            shards: vec![ManaCostShard::X, ManaCostShard::Blue],
            generic: 0,
        })
        .id();
    for _ in 0..islands {
        scenario.add_basic_land(P0, ManaColor::Blue);
    }
    scenario.with_library_top(P0, &["Wastes", "Wastes", "Wastes"]);
    extra(&mut scenario);
    (scenario.build(), spell)
}

fn offers_cast(runner: &engine::game::scenario::GameRunner, spell: ObjectId) -> bool {
    legal_actions(runner.state()).iter().any(
        |action| matches!(action, GameAction::CastSpell { object_id, .. } if *object_id == spell),
    )
}

/// Cast the X-minimum spell, announce X = 1 (the only affordable legal X) and
/// resolve it: it draws one card.
fn cast_at_minimum_x(runner: &mut engine::game::scenario::GameRunner, spell: ObjectId) {
    let card_id = runner.state().objects[&spell].card_id;
    let hand_before = runner.state().players[0].hand.len();
    runner
        .act(GameAction::CastSpell {
            object_id: spell,
            card_id,
            targets: vec![],
            payment_mode: CastPaymentMode::Auto,
        })
        .expect("the offered cast is accepted");
    match &runner.state().waiting_for {
        WaitingFor::ChooseXValue { min, max, .. } => {
            assert_eq!((*min, *max), (1, 1), "X = 1 is the only affordable legal X");
        }
        other => panic!("expected the X announcement, got {other:?}"),
    }
    runner
        .act(GameAction::ChooseX { value: 1 })
        .expect("X = 1 is legal");
    runner.advance_until_stack_empty();
    assert_eq!(runner.state().objects[&spell].zone, Zone::Graveyard);
    assert_eq!(
        runner.state().players[0].hand.len(),
        hand_before,
        "the spell left the hand and drew one card"
    );
}

#[test]
fn x_minimum_spell_not_castable_when_only_x_zero_is_affordable() {
    let (runner, spell) = draw_x_board(1, |_| {});
    assert!(!offers_cast(&runner, spell));
    assert!(!engine::game::casting::can_cast_object_now(
        runner.state(),
        P0,
        spell
    ));
}

#[test]
fn x_minimum_spell_castable_when_the_minimum_is_affordable() {
    let (mut runner, spell) = draw_x_board(2, |_| {});
    assert!(offers_cast(&runner, spell));
    cast_at_minimum_x(&mut runner, spell);
}

/// Goblin Electromancer (Oracle text verified against Scryfall).
const GOBLIN_ELECTROMANCER: &str = "Instant and sorcery spells you cast cost {1} less to cast.";
/// Trinisphere (Oracle text verified against Scryfall).
const TRINISPHERE: &str = "As long as this artifact is untapped, each spell that would cost less than three mana to cast costs three mana to cast. (Additional mana in the cost may be paid with any color of mana or colorless mana. For example, a spell that would cost {1}{B} to cast costs {2}{B} to cast instead.)";

#[test]
fn x_minimum_spell_priced_after_its_reductions() {
    // CR 601.2f: {X}{U} with X = 1 is {1}{U}, and the reducer takes the {1}:
    // one Island pays it.
    let (mut runner, spell) = draw_x_board(1, |scenario| {
        scenario.add_creature_from_oracle(P0, "Goblin Electromancer", 2, 2, GOBLIN_ELECTROMANCER);
    });
    assert!(
        offers_cast(&runner, spell),
        "X = 1 costs {{U}} beside the reducer"
    );
    cast_at_minimum_x(&mut runner, spell);
}

#[test]
fn x_minimum_spell_priced_with_its_floor() {
    // CR 601.2f: the floor reads the total with X = 1, {1}{U}, and raises it to
    // three mana, so two Islands can't pay it and three can.
    let trinisphere = |scenario: &mut GameScenario| {
        scenario.add_artifact_from_oracle(P0, "Trinisphere", TRINISPHERE);
    };
    let (runner, spell) = draw_x_board(2, trinisphere);
    assert!(!offers_cast(&runner, spell));
    let (runner, spell) = draw_x_board(3, trinisphere);
    assert!(offers_cast(&runner, spell));
}

/// Ruthless Technomancer (Oracle text verified against Scryfall).
const RUTHLESS_TECHNOMANCER: &str = "When this creature enters, you may sacrifice another creature you control. If you do, create a number of Treasure tokens equal to that creature's power.\n{2}{B}, Sacrifice X artifacts: Return target creature card with power X or less from your graveyard to the battlefield. X can't be 0.";

fn technomancer_board(artifacts: usize) -> (engine::game::scenario::GameRunner, ObjectId) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let technomancer = scenario
        .add_creature_from_oracle(P0, "Ruthless Technomancer", 2, 4, RUTHLESS_TECHNOMANCER)
        .id();
    for _ in 0..3 {
        scenario.add_basic_land(P0, ManaColor::Black);
    }
    for _ in 0..artifacts {
        scenario.add_artifact_from_oracle(P0, "Ornithopter Husk", "");
    }
    scenario.add_creature_to_graveyard(P0, "Grizzly Bears", 2, 2);
    (scenario.build(), technomancer)
}

fn min_x_ability_index(runner: &engine::game::scenario::GameRunner, source: ObjectId) -> usize {
    runner.state().objects[&source]
        .abilities
        .iter()
        .position(|ability| ability.min_x_value == 1)
        .expect("the reanimation ability carries X's minimum")
}

#[test]
fn x_minimum_sacrifice_x_not_offered_with_nothing_to_sacrifice() {
    let (runner, technomancer) = technomancer_board(0);
    let index = min_x_ability_index(&runner, technomancer);
    assert!(
        !offers_activation(&runner, technomancer, index),
        "X = 1 needs an artifact to sacrifice"
    );
    let (runner, technomancer) = technomancer_board(1);
    assert!(offers_activation(&runner, technomancer, index));
}

/// Katara, Water Tribe's Hope (Oracle text verified against Scryfall).
const KATARA: &str = "Vigilance\nWhen Katara enters, create a 1/1 white Ally creature token.\nWaterbend {X}: Creatures you control have base power and toughness X/X until end of turn. X can't be 0. Activate only during your turn. (While paying a waterbend cost, you can tap your artifacts and creatures to help. Each one pays for {1}.)";

#[test]
fn x_minimum_waterbend_priced_at_the_minimum() {
    let board = |islands: usize| {
        let mut scenario = GameScenario::new();
        scenario.at_phase(Phase::PreCombatMain);
        let katara = scenario
            .add_creature_from_oracle(P0, "Katara, Water Tribe's Hope", 3, 3, KATARA)
            .id();
        for _ in 0..islands {
            scenario.add_basic_land(P0, ManaColor::Blue);
        }
        let mut runner = scenario.build();
        // Tapped, so Katara can't help pay the waterbend cost herself.
        runner.state_mut().objects.get_mut(&katara).unwrap().tapped = true;
        let index = min_x_ability_index(&runner, katara);
        (runner, katara, index)
    };
    let (runner, katara, index) = board(0);
    assert!(
        !offers_activation(&runner, katara, index),
        "waterbend {{X}} at X = 1 needs one mana or one untapped creature or artifact"
    );
    let (runner, katara, index) = board(1);
    assert!(offers_activation(&runner, katara, index));
}

/// Omniscience (Oracle text verified against Scryfall).
const OMNISCIENCE: &str = "You may cast spells from your hand without paying their mana costs.";

fn with_omniscience(scenario: &mut GameScenario) {
    scenario.add_enchantment_from_oracle(P0, "Omniscience", OMNISCIENCE);
}

#[test]
fn x_minimum_spell_not_castable_without_paying_its_mana_cost() {
    // CR 107.3b: cast without paying its mana cost, the spell's X is 0, which
    // "X can't be 0" forbids, so the free cast isn't offered.
    let (runner, spell) = draw_x_board(0, with_omniscience);
    assert!(!offers_cast(&runner, spell));
    // Paying the mana cost with X = 1 is still offered beside Omniscience.
    let (runner, spell) = draw_x_board(2, with_omniscience);
    assert!(offers_cast(&runner, spell));
}

#[test]
fn free_cast_of_a_spell_without_x_is_still_offered() {
    // Control for the board above: Omniscience offers a spell with no {X}.
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, "Plain Draw", false, "Draw a card.")
        .with_mana_cost(ManaCost::Cost {
            shards: vec![ManaCostShard::Blue],
            generic: 1,
        })
        .id();
    with_omniscience(&mut scenario);
    scenario.with_library_top(P0, &["Wastes"]);
    let runner = scenario.build();
    assert!(offers_cast(&runner, spell));
}

/// Synthetic (labelled): X only in a kicker, with a stated minimum (the
/// Thieving Skydiver shape on a sorcery).
const KICKER_X_MINIMUM: &str =
    "Kicker {X}\nDraw a card. If this spell was kicked, draw X cards instead.\nX can't be 0.";

#[test]
fn x_minimum_in_a_kicker_leaves_the_free_cast_offered() {
    // The mana cost has no {X}, so CR 107.3b doesn't apply: unkicked, the
    // spell is cast free under Omniscience.
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, "Kicked Draw", false, KICKER_X_MINIMUM)
        .with_mana_cost(ManaCost::Cost {
            shards: vec![ManaCostShard::Blue],
            generic: 0,
        })
        .id();
    with_omniscience(&mut scenario);
    scenario.with_library_top(P0, &["Wastes"; 3]);
    let runner = scenario.build();
    assert!(offers_cast(&runner, spell));
}
