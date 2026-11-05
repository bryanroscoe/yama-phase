//! CR 107.14 + CR 118.3 + CR 118.9: a card that may be cast from exile "by
//! paying an amount of {E} equal to its mana value rather than paying its mana
//! cost" (Amped Raptor) costs that much energy, and a player can't pay more {E}
//! than they have. The cast is offered only when the energy is there, so the
//! engine never has to answer "Not enough energy" to an action it offered.

use engine::ai_support::legal_actions;
use engine::game::scenario::{GameRunner, GameScenario, P0};
use engine::types::ability::CastingPermission;
use engine::types::actions::GameAction;
use engine::types::game_state::CastPaymentMode;
use engine::types::identifiers::ObjectId;
use engine::types::mana::{ManaColor, ManaCost, ManaCostShard};
use engine::types::phase::Phase;
use engine::types::zones::Zone;

/// Amped Raptor (Oracle text verified against Scryfall).
const AMPED_RAPTOR: &str = "First strike\nWhen this creature enters, you get {E}{E} (two energy counters). Then if you cast it from your hand, exile cards from the top of your library until you exile a nonland card. You may cast that card by paying an amount of {E} equal to its mana value rather than paying its mana cost.";

fn offers_cast(runner: &GameRunner, card: ObjectId) -> bool {
    legal_actions(runner.state()).iter().any(
        |action| matches!(action, GameAction::CastSpell { object_id, .. } if *object_id == card),
    )
}

/// An exiled sorcery of mana value `mana_value` carrying the energy-cost
/// permission, with `energy` for its owner.
fn exiled_energy_cast(mana_value: u32, energy: u32) -> (GameRunner, ObjectId) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let card = scenario
        .add_spell_to_exile(P0, "Exiled Sorcery", false)
        .with_mana_cost(ManaCost::generic(mana_value))
        .id();
    let mut runner = scenario.build();
    runner.state_mut().players[0].energy = energy;
    runner
        .state_mut()
        .objects
        .get_mut(&card)
        .unwrap()
        .casting_permissions
        .push(CastingPermission::ExileWithEnergyCost);
    (runner, card)
}

#[test]
fn energy_cast_not_offered_without_enough_energy() {
    let (runner, card) = exiled_energy_cast(3, 2);
    assert!(!offers_cast(&runner, card), "2 {{E}} can't pay 3");
}

#[test]
fn energy_cast_offered_with_exactly_enough_and_spends_it_all() {
    let (mut runner, card) = exiled_energy_cast(3, 3);
    assert!(offers_cast(&runner, card));
    let card_id = runner.state().objects[&card].card_id;
    runner
        .act(GameAction::CastSpell {
            object_id: card,
            card_id,
            targets: vec![],
            payment_mode: CastPaymentMode::Auto,
        })
        .expect("the offered cast is accepted");
    assert_eq!(runner.state().players[0].energy, 0);
    assert_eq!(runner.state().objects[&card].zone, Zone::Stack);
}

/// Amped Raptor's trigger exiles a card of mana value `mana_value` while its
/// controller has 2 {E}. Returns the runner after the trigger resolved and the
/// exiled card.
fn amped_raptor_board(mana_value: u32) -> (GameRunner, ObjectId) {
    {
        let mut scenario = GameScenario::new();
        scenario.at_phase(Phase::PreCombatMain);
        for _ in 0..2 {
            scenario.add_basic_land(P0, ManaColor::Red);
        }
        // Library, top first: a land, then the nonland card, then filler.
        for _ in 0..6 {
            scenario.add_land_to_library_top(P0, "Wastes");
        }
        let hit = scenario
            .add_spell_to_library_top(P0, "Exiled Hit", false)
            .with_mana_cost(ManaCost::generic(mana_value))
            .id();
        scenario.add_land_to_library_top(P0, "Mountain");
        let raptor = scenario
            .add_creature_to_hand_from_oracle(P0, "Amped Raptor", 2, 1, AMPED_RAPTOR)
            .with_mana_cost(ManaCost::Cost {
                shards: vec![ManaCostShard::Red],
                generic: 1,
            })
            .id();
        let mut runner = scenario.build();
        let card_id = runner.state().objects[&raptor].card_id;
        runner
            .act(GameAction::CastSpell {
                object_id: raptor,
                card_id,
                targets: vec![],
                payment_mode: CastPaymentMode::Auto,
            })
            .expect("Amped Raptor is cast");
        runner.advance_until_stack_empty();
        assert_eq!(runner.state().players[0].energy, 2);
        assert_eq!(runner.state().objects[&hit].zone, Zone::Exile);
        (runner, hit)
    }
}

#[test]
fn amped_raptor_does_not_offer_an_exiled_card_its_energy_cannot_pay() {
    // Coveted Jewel's shape: mana value 6 against 2 {E}.
    let (runner, hit) = amped_raptor_board(6);
    assert!(!offers_cast(&runner, hit));
    assert_eq!(runner.state().players[0].energy, 2);
}

#[test]
fn amped_raptor_offers_an_exiled_card_its_energy_can_pay() {
    // Swords to Plowshares' shape: mana value 1 against 2 {E}. The energy gate
    // must not hide an affordable card. (When the offer is made is a separate,
    // open defect: CR 608.2g limits this cast to the trigger's resolution.)
    let (runner, hit) = amped_raptor_board(1);
    assert!(offers_cast(&runner, hit));
}
