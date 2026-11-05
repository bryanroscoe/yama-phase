//! CR 601.2b + CR 601.2f-h + CR 118.3: a spell's REQUIRED additional cost is
//! part of its total cost, so a spell whose required cost needs an object the
//! caster doesn't have (a permanent to sacrifice, a card to discard or exile)
//! can't be cast, and is not offered. CR 701.21a: a player sacrifices only a
//! permanent they control.

use engine::ai_support::legal_actions;
use engine::game::game_object::{PhaseOutCause, PhaseStatus};
use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::types::actions::GameAction;
use engine::types::game_state::{CastPaymentMode, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::mana::{ManaColor, ManaCost, ManaCostShard};
use engine::types::phase::Phase;
use engine::types::zones::Zone;

/// Deadly Dispute (Oracle text verified against Scryfall).
const DEADLY_DISPUTE: &str = "As an additional cost to cast this spell, sacrifice an artifact or creature.\nDraw two cards and create a Treasure token. (It's an artifact with \"{T}, Sacrifice this token: Add one mana of any color.\")";
/// Village Rites (Oracle text verified against Scryfall).
const VILLAGE_RITES: &str =
    "As an additional cost to cast this spell, sacrifice a creature.\nDraw two cards.";
/// Synthetic (labelled) required discard: "As an additional cost to cast this
/// spell, discard a card." The class shares the same gate.
const DISCARD_REQUIRED: &str =
    "As an additional cost to cast this spell, discard a card.\nDraw two cards.";
const MILLSTONE: &str = "{2}, {T}: Target player mills two cards.";
const TREASURE: &str = "{T}, Sacrifice this artifact: Add one mana of any color.";

fn offers_cast(runner: &GameRunner, spell: ObjectId) -> bool {
    legal_actions(runner.state()).iter().any(
        |action| matches!(action, GameAction::CastSpell { object_id, .. } if *object_id == spell),
    )
}

struct DisputeBoard {
    runner: GameRunner,
    dispute: ObjectId,
    swamps: [ObjectId; 2],
    opponent_bears: ObjectId,
    opponent_millstone: ObjectId,
}

/// Seat 0: Swamp ×2 and Deadly Dispute in hand (plus `own`'s permanents).
/// Seat 1: Grizzly Bears and Millstone.
fn dispute_board(own: impl FnOnce(&mut GameScenario)) -> DisputeBoard {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let dispute = scenario
        .add_spell_to_hand_from_oracle(P0, "Deadly Dispute", true, DEADLY_DISPUTE)
        .with_mana_cost(ManaCost::Cost {
            shards: vec![ManaCostShard::Black],
            generic: 1,
        })
        .id();
    let swamps = [
        scenario.add_basic_land(P0, ManaColor::Black),
        scenario.add_basic_land(P0, ManaColor::Black),
    ];
    scenario.with_library_top(P0, &["Wastes"; 8]);
    let opponent_bears = scenario.add_creature(P1, "Grizzly Bears", 2, 2).id();
    let opponent_millstone = scenario
        .add_artifact_from_oracle(P1, "Millstone", MILLSTONE)
        .id();
    own(&mut scenario);
    DisputeBoard {
        runner: scenario.build(),
        dispute,
        swamps,
        opponent_bears,
        opponent_millstone,
    }
}

#[test]
fn required_sacrifice_with_nothing_to_sacrifice_is_not_castable() {
    let DisputeBoard {
        mut runner,
        dispute,
        swamps,
        opponent_bears,
        opponent_millstone,
    } = dispute_board(|_| {});
    assert!(
        !offers_cast(&runner, dispute),
        "the opponent's Bears and Millstone are not seat 0's to sacrifice"
    );
    assert!(!engine::game::casting::can_cast_object_now(
        runner.state(),
        P0,
        dispute
    ));

    let card_id = runner.state().objects[&dispute].card_id;
    let result = runner.act(GameAction::CastSpell {
        object_id: dispute,
        card_id,
        targets: vec![],
        payment_mode: CastPaymentMode::Auto,
    });
    // CR 733.1: whether refused outright or reversed, nothing changes.
    let state = runner.state();
    assert!(
        result.is_err()
            || matches!(state.waiting_for, WaitingFor::Priority { player } if player == P0),
        "a submitted cast must not leave a cost prompt: {:?}",
        state.waiting_for
    );
    assert_eq!(state.objects[&dispute].zone, Zone::Hand);
    assert!(swamps.iter().all(|swamp| !state.objects[swamp].tapped));
    assert_eq!(state.objects[&opponent_bears].zone, Zone::Battlefield);
    assert_eq!(state.objects[&opponent_millstone].zone, Zone::Battlefield);
}

#[test]
fn required_sacrifice_offered_with_own_treasure_and_sacrifices_only_it() {
    let mut treasure = ObjectId(0);
    let DisputeBoard {
        mut runner,
        dispute,
        opponent_bears,
        opponent_millstone,
        ..
    } = dispute_board(|scenario| {
        let mut builder = scenario.add_artifact_from_oracle(P0, "Treasure", TREASURE);
        builder.with_subtypes(vec!["Treasure"]);
        treasure = builder.id();
    });
    runner
        .state_mut()
        .objects
        .get_mut(&treasure)
        .unwrap()
        .is_token = true;
    assert!(offers_cast(&runner, dispute));

    let outcome = runner.cast(dispute).sacrifice_with(&[treasure]).resolve();
    let state = outcome.state();
    assert_eq!(state.objects[&dispute].zone, Zone::Graveyard);
    assert!(
        !state.battlefield.contains(&treasure),
        "the original Treasure paid the additional cost"
    );
    assert_eq!(state.players[0].hand.len(), 2, "Deadly Dispute drew two");
    let own_treasures = state
        .battlefield
        .iter()
        .filter_map(|id| state.objects.get(id))
        .filter(|obj| obj.controller == P0 && obj.name == "Treasure")
        .count();
    assert_eq!(own_treasures, 1, "one new Treasure");
    assert_eq!(state.objects[&opponent_bears].zone, Zone::Battlefield);
    assert_eq!(state.objects[&opponent_millstone].zone, Zone::Battlefield);
}

#[test]
fn required_sacrifice_offered_with_own_creature() {
    let DisputeBoard {
        runner, dispute, ..
    } = dispute_board(|scenario| {
        scenario.add_creature(P0, "Grizzly Bears", 2, 2);
    });
    assert!(offers_cast(&runner, dispute));
}

#[test]
fn required_sacrifice_not_castable_with_only_a_land() {
    // The Swamps are seat 0's only permanents: lands aren't artifacts or creatures.
    let DisputeBoard {
        runner, dispute, ..
    } = dispute_board(|_| {});
    assert!(!offers_cast(&runner, dispute));
}

#[test]
fn required_sacrifice_offered_with_an_artifact_land() {
    let DisputeBoard {
        runner, dispute, ..
    } = dispute_board(|scenario| {
        scenario
            .add_land_from_oracle(P0, "Vault of Whispers", "{T}: Add {B}.")
            .as_artifact_land();
    });
    assert!(offers_cast(&runner, dispute));
}

#[test]
fn required_sacrifice_not_castable_when_the_only_creature_is_phased_out() {
    let mut bears = ObjectId(0);
    let DisputeBoard {
        mut runner,
        dispute,
        ..
    } = dispute_board(|scenario| {
        bears = scenario.add_creature(P0, "Grizzly Bears", 2, 2).id();
    });
    // CR 702.26b: a phased-out permanent is treated as though it doesn't exist.
    runner
        .state_mut()
        .objects
        .get_mut(&bears)
        .unwrap()
        .phase_status = PhaseStatus::PhasedOut {
        cause: PhaseOutCause::Directly,
    };
    assert!(!offers_cast(&runner, dispute));
}

fn single_spell_board(oracle: &str, name: &str, with_creature: bool) -> (GameRunner, ObjectId) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, name, true, oracle)
        .with_mana_cost(ManaCost::Cost {
            shards: vec![ManaCostShard::Black],
            generic: 0,
        })
        .id();
    scenario.add_basic_land(P0, ManaColor::Black);
    scenario.add_creature(P1, "Grizzly Bears", 2, 2);
    if with_creature {
        scenario.add_creature(P0, "Llanowar Elves", 1, 1);
    }
    (scenario.build(), spell)
}

#[test]
fn village_rites_needs_a_creature_of_its_casters() {
    let (runner, rites) = single_spell_board(VILLAGE_RITES, "Village Rites", false);
    assert!(!offers_cast(&runner, rites));
    let (runner, rites) = single_spell_board(VILLAGE_RITES, "Village Rites", true);
    assert!(offers_cast(&runner, rites));
}

#[test]
fn required_discard_with_an_otherwise_empty_hand_is_not_castable() {
    let (runner, spell) = single_spell_board(DISCARD_REQUIRED, "Required Discard", false);
    assert_eq!(
        runner.state().players[0].hand.len(),
        1,
        "only the spell itself"
    );
    assert!(
        !offers_cast(&runner, spell),
        "the spell being cast can't be discarded to pay its own cost"
    );
}

/// Mind Stone (Oracle text verified against Scryfall).
const MIND_STONE: &str = "{T}: Add {C}.\n{1}, {T}, Sacrifice this artifact: Draw a card.";
/// Ashnod's Altar (Oracle text verified against Scryfall).
const ASHNODS_ALTAR: &str = "Sacrifice a creature: Add {C}{C}.";

/// Seat 0: `swamps` Swamps, Deadly Dispute in hand, plus `own`'s permanents.
fn lean_dispute_board(
    swamps: usize,
    own: impl FnOnce(&mut GameScenario, &mut Vec<ObjectId>),
) -> (GameRunner, ObjectId, Vec<ObjectId>) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let dispute = scenario
        .add_spell_to_hand_from_oracle(P0, "Deadly Dispute", true, DEADLY_DISPUTE)
        .with_mana_cost(ManaCost::Cost {
            shards: vec![ManaCostShard::Black],
            generic: 1,
        })
        .id();
    for _ in 0..swamps {
        scenario.add_basic_land(P0, ManaColor::Black);
    }
    scenario.with_library_top(P0, &["Wastes"; 8]);
    let mut owned = Vec::new();
    own(&mut scenario, &mut owned);
    (scenario.build(), dispute, owned)
}

fn add_treasure(scenario: &mut GameScenario, owned: &mut Vec<ObjectId>) {
    let mut builder = scenario.add_artifact_from_oracle(P0, "Treasure", TREASURE);
    builder.with_subtypes(vec!["Treasure"]);
    owned.push(builder.id());
}

#[test]
fn required_sacrifice_treasure_cannot_also_pay_mana() {
    // CR 118.10: the only Treasure can be sacrificed for Deadly Dispute or for
    // mana, not both, and one Swamp can't pay {1}{B} alone.
    let (runner, dispute, _) = lean_dispute_board(1, add_treasure);
    assert!(!offers_cast(&runner, dispute));
    assert!(!engine::game::casting::can_cast_object_now(
        runner.state(),
        P0,
        dispute
    ));
}

#[test]
fn required_sacrifice_treasure_kept_while_lands_pay_the_mana() {
    let (runner, dispute, _) = lean_dispute_board(2, add_treasure);
    assert!(offers_cast(&runner, dispute));
}

#[test]
fn required_sacrifice_mana_rock_taps_for_mana_then_is_sacrificed() {
    // CR 601.2g-h: Mind Stone taps for {C} while mana abilities are activated,
    // then is sacrificed as the additional cost.
    let (mut runner, dispute, owned) = lean_dispute_board(1, |scenario, owned| {
        owned.push(
            scenario
                .add_artifact_from_oracle(P0, "Mind Stone", MIND_STONE)
                .id(),
        );
    });
    assert!(offers_cast(&runner, dispute));
    let mind_stone = owned[0];
    let outcome = runner.cast(dispute).sacrifice_with(&[mind_stone]).resolve();
    let state = outcome.state();
    assert_eq!(state.objects[&dispute].zone, Zone::Graveyard);
    assert!(!state.battlefield.contains(&mind_stone));
    assert_eq!(state.players[0].hand.len(), 2, "Deadly Dispute drew two");
}

#[test]
fn required_sacrifice_altar_feeds_one_creature_and_is_itself_sacrificed() {
    // CR 601.2g-h: Ashnod's Altar sacrifices the Bears for {C}{C} while mana
    // abilities are activated, then the Altar (an artifact) is sacrificed as
    // the additional cost; keeping the Bears instead would leave one Swamp.
    let (runner, dispute, _) = lean_dispute_board(1, |scenario, owned| {
        owned.push(
            scenario
                .add_artifact_from_oracle(P0, "Ashnod's Altar", ASHNODS_ALTAR)
                .id(),
        );
        owned.push(scenario.add_creature(P0, "Grizzly Bears", 2, 2).id());
    });
    assert!(offers_cast(&runner, dispute));
}

#[test]
fn required_sacrifice_altar_fodder_cannot_pay_twice() {
    // CR 118.10: the Bears is the only artifact or creature to sacrifice, and
    // the only fodder for a (synthetic, labelled) enchantment Altar, so it
    // can pay the additional cost or the Altar's mana, not both.
    let (runner, dispute, _) = lean_dispute_board(1, |scenario, owned| {
        owned.push(scenario.add_creature(P0, "Grizzly Bears", 2, 2).id());
        scenario.add_enchantment_from_oracle(P0, "Enchanted Altar", ASHNODS_ALTAR);
    });
    assert!(!offers_cast(&runner, dispute));
}

/// Synthetic (labelled) required sacrifice whose mana total the Treasures
/// alone must pay: "As an additional cost to cast this spell, sacrifice an
/// artifact." with a mana cost of `{generic}{B}`.
const SACRIFICE_AN_ARTIFACT: &str =
    "As an additional cost to cast this spell, sacrifice an artifact.\nDraw a card.";

fn treasure_hoard_board(generic: u32) -> (GameRunner, ObjectId) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, "Hoard Rite", true, SACRIFICE_AN_ARTIFACT)
        .with_mana_cost(ManaCost::Cost {
            shards: vec![ManaCostShard::Black],
            generic,
        })
        .id();
    scenario.with_library_top(P0, &["Wastes"; 2]);
    let mut owned = Vec::new();
    for _ in 0..9 {
        add_treasure(&mut scenario, &mut owned);
    }
    let mut runner = scenario.build();
    for treasure in owned {
        runner
            .state_mut()
            .objects
            .get_mut(&treasure)
            .unwrap()
            .is_token = true;
    }
    (runner, spell)
}

#[test]
fn required_sacrifice_of_nine_identical_treasures_is_judged_once() {
    // CR 118.10: one of the nine Treasures pays the sacrifice, so eight are
    // left for mana. {8}{B} needs all nine: not offered. {7}{B} needs eight:
    // offered. The Treasures are interchangeable, so one probe decides.
    let (runner, spell) = treasure_hoard_board(8);
    assert!(!offers_cast(&runner, spell));
    let (runner, spell) = treasure_hoard_board(7);
    assert!(offers_cast(&runner, spell));
}

/// Skirge Familiar (Oracle text verified against Scryfall).
const SKIRGE_FAMILIAR: &str = "Flying\nDiscard a card: Add {B}.";

fn skirge_board(other_cards: usize) -> (GameRunner, ObjectId) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, "Required Discard", true, DISCARD_REQUIRED)
        .with_mana_cost(ManaCost::Cost {
            shards: vec![ManaCostShard::Black],
            generic: 0,
        })
        .id();
    scenario.add_creature_from_oracle(P0, "Skirge Familiar", 3, 2, SKIRGE_FAMILIAR);
    for _ in 0..other_cards {
        scenario.add_card_to_hand(P0, "Wastes");
    }
    scenario.with_library_top(P0, &["Wastes"; 2]);
    (scenario.build(), spell)
}

#[test]
fn required_discard_card_cannot_also_pay_skirge_familiar() {
    // CR 118.10: with one other card, it pays the discard or Skirge
    // Familiar's {B}, not both; with two, one of each.
    let (runner, spell) = skirge_board(1);
    assert!(!offers_cast(&runner, spell));
    let (runner, spell) = skirge_board(2);
    assert!(offers_cast(&runner, spell));
}

/// Synthetic (labelled): a required life payment beside a mana source that
/// costs life: "Pay 1 life: Add {B}." pays a {B} spell's mana.
const PAY_LIFE_REQUIRED: &str =
    "As an additional cost to cast this spell, pay 3 life.\nDraw a card.";
const LIFE_FOR_MANA: &str = "Pay 1 life: Add {B}.";

#[test]
fn required_life_payment_and_life_for_mana_share_one_total() {
    // CR 119.4: with 3 life, paying 3 for the cost leaves none for the {B};
    // with 4 life both are paid.
    let board = |life: i32| {
        let mut scenario = GameScenario::new();
        scenario.at_phase(Phase::PreCombatMain);
        let spell = scenario
            .add_spell_to_hand_from_oracle(P0, "Life Rite", true, PAY_LIFE_REQUIRED)
            .with_mana_cost(ManaCost::Cost {
                shards: vec![ManaCostShard::Black],
                generic: 0,
            })
            .id();
        scenario.add_enchantment_from_oracle(P0, "Life Font", LIFE_FOR_MANA);
        scenario.with_life(P0, life);
        scenario.with_library_top(P0, &["Wastes"; 2]);
        (scenario.build(), spell)
    };
    let (runner, spell) = board(3);
    assert!(!offers_cast(&runner, spell));
    let (runner, spell) = board(4);
    assert!(offers_cast(&runner, spell));
}

/// Synthetic (labelled): a required life payment on a spell whose mana is
/// paid with life too.
const PAY_TWO_LIFE_REQUIRED: &str =
    "As an additional cost to cast this spell, pay 2 life.\nDraw a card.";
/// K'rrik, Son of Yawgmoth (Oracle text verified against Scryfall), without
/// its mana-cost reminder line.
const KRRIK: &str = "Lifelink\nFor each {B} in a cost, you may pay 2 life rather than pay that mana.\nWhenever you cast a black spell, put a +1/+1 counter on K'rrik.";

fn life_paid_mana_board(
    life: i32,
    mana_cost: ManaCost,
    extra: impl FnOnce(&mut GameScenario),
) -> (GameRunner, ObjectId) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, "Blood Rite", true, PAY_TWO_LIFE_REQUIRED)
        .with_mana_cost(mana_cost)
        .id();
    extra(&mut scenario);
    scenario.with_life(P0, life);
    scenario.with_library_top(P0, &["Wastes"; 2]);
    (scenario.build(), spell)
}

#[test]
fn required_life_payment_and_phyrexian_mana_share_one_total() {
    // CR 119.4 + CR 107.4f: with no black source, {B/P} costs 2 life beside the
    // required 2: at 3 life the second payment can't be made (1 < 2); at 4
    // life both are paid, down to exactly 0 (CR 119.4 allows that).
    let phyrexian = || ManaCost::Cost {
        shards: vec![ManaCostShard::PhyrexianBlack],
        generic: 0,
    };
    let (runner, spell) = life_paid_mana_board(3, phyrexian(), |_| {});
    assert!(!offers_cast(&runner, spell));
    let (runner, spell) = life_paid_mana_board(4, phyrexian(), |_| {});
    assert!(offers_cast(&runner, spell));
}

#[test]
fn required_life_payment_and_krrik_permission_share_one_total() {
    // CR 119.4: K'rrik lets the {B} be paid with 2 life; with no black source
    // that 2 and the required 2 need 4 life.
    let black = || ManaCost::Cost {
        shards: vec![ManaCostShard::Black],
        generic: 0,
    };
    let krrik = |scenario: &mut GameScenario| {
        scenario.add_creature_from_oracle(P0, "K'rrik, Son of Yawgmoth", 2, 2, KRRIK);
    };
    let (runner, spell) = life_paid_mana_board(3, black(), krrik);
    assert!(!offers_cast(&runner, spell));
    let (runner, spell) = life_paid_mana_board(4, black(), krrik);
    assert!(offers_cast(&runner, spell));
}

/// Crypt of Agadeem (Oracle text verified against Scryfall).
const CRYPT_OF_AGADEEM: &str = "This land enters tapped.\n{T}: Add {B}.\n{2}, {T}: Add {B} for each black creature card in your graveyard.";
/// Synthetic (labelled) required graveyard exile.
const EXILE_A_CREATURE_CARD: &str = "As an additional cost to cast this spell, exile a creature card from your graveyard.\nDraw a card.";

/// Rubble Rouser (Oracle text verified against Scryfall).
const RUBBLE_ROUSER: &str = "When this creature enters, you may discard a card. If you do, draw a card.\n{T}, Exile a card from your graveyard: Add {R}. When you do, this creature deals 1 damage to each opponent.";

/// Two Swamps, Crypt of Agadeem and four black creature cards in the
/// graveyard; a {3}{B} spell that exiles one of them as its cost. A tapped
/// Rubble Rouser, whose mana ability exiles graveyard cards, makes the
/// affordability check reserve the kept card.
fn crypt_board() -> (GameRunner, ObjectId, Vec<ObjectId>) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, "Grave Rite", true, EXILE_A_CREATURE_CARD)
        .with_mana_cost(ManaCost::Cost {
            shards: vec![ManaCostShard::Black],
            generic: 3,
        })
        .id();
    scenario.add_basic_land(P0, ManaColor::Black);
    scenario.add_basic_land(P0, ManaColor::Black);
    scenario.add_land_from_oracle(P0, "Crypt of Agadeem", CRYPT_OF_AGADEEM);
    let cards = (0..4)
        .map(|_| {
            scenario
                .add_creature_to_graveyard(P0, "Vampire Card", 2, 2)
                .with_color(vec![ManaColor::Black])
                .id()
        })
        .collect();
    let rouser = scenario
        .add_creature_from_oracle(P0, "Rubble Rouser", 1, 4, RUBBLE_ROUSER)
        .id();
    scenario.with_library_top(P0, &["Wastes"; 2]);
    let mut runner = scenario.build();
    runner.state_mut().objects.get_mut(&rouser).unwrap().tapped = true;
    (runner, spell, cards)
}

#[test]
fn required_graveyard_exile_card_still_counts_for_mana() {
    // CR 601.2g-h: mana abilities are activated before the exile is paid, so
    // Crypt of Agadeem counts all four black creature cards ({2} from the
    // Swamps buys {B}{B}{B}{B}), and the one kept for the exile pays it after.
    // Counting only three would leave three mana for a {3}{B} spell.
    let (runner, spell, _) = crypt_board();
    assert!(engine::game::casting::can_cast_object_now(
        runner.state(),
        P0,
        spell
    ));
    assert!(offers_cast(&runner, spell));
}

/// Harrow (Oracle text verified against Scryfall).
const HARROW: &str = "As an additional cost to cast this spell, sacrifice a land.\nSearch your library for up to two basic land cards, put them onto the battlefield, then shuffle.";
/// Crystal Vein (Oracle text verified against Scryfall).
const CRYSTAL_VEIN: &str = "{T}: Add {C}.\n{T}, Sacrifice this land: Add {C}{C}.";

/// A land-sacrificing spell costing `{generic}{G}` (Harrow is `{2}{G}`), with
/// `veins` Crystal Veins (the first `tapped_veins` of them tapped) and
/// `green` {G} floating.
fn vein_board(
    generic: u32,
    veins: usize,
    tapped_veins: usize,
    green: usize,
) -> (GameRunner, ObjectId) {
    use engine::types::mana::{ManaType, ManaUnit};
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, "Harrow", true, HARROW)
        .with_mana_cost(ManaCost::Cost {
            shards: vec![ManaCostShard::Green],
            generic,
        })
        .id();
    let veins: Vec<ObjectId> = (0..veins)
        .map(|_| {
            scenario
                .add_land_from_oracle(P0, "Crystal Vein", CRYSTAL_VEIN)
                .id()
        })
        .collect();
    scenario.with_mana_pool(
        P0,
        (0..green)
            .map(|_| ManaUnit::new(ManaType::Green, ObjectId(0), false, vec![]))
            .collect(),
    );
    scenario.with_library_top(P0, &["Forest"; 2]);
    let mut runner = scenario.build();
    for vein in veins.iter().take(tapped_veins) {
        runner.state_mut().objects.get_mut(vein).unwrap().tapped = true;
    }
    (runner, spell)
}

#[test]
fn required_sacrifice_land_cannot_also_sacrifice_itself_for_mana() {
    // CR 118.10: the only land is kept for Harrow's sacrifice, so it can tap
    // for {C} but not sacrifice itself for {C}{C}: {G} + {C} can't pay {2}{G}.
    // (The mana check doesn't count Crystal Vein's {C}{C} at all yet, so the
    // Grove Vein test below is the one that pins the reservation.)
    let (runner, spell) = vein_board(2, 1, 0, 1);
    assert!(!offers_cast(&runner, spell));
}

#[test]
fn required_sacrifice_land_still_taps_for_its_renewable_mana() {
    // The kept Crystal Vein taps for {C} (CR 601.2g) before it is sacrificed:
    // {G}{G} floating + {C} pays {2}{G}.
    let (runner, spell) = vein_board(2, 1, 0, 2);
    assert!(offers_cast(&runner, spell));
}

#[test]
fn required_sacrifice_kept_vein_leaves_its_twin_usable() {
    // Keeping one Crystal Vein for Harrow's sacrifice reserves only that Vein:
    // both still tap for {C} (the kept one before it is sacrificed), so {G}
    // floating + {C}{C} pays {2}{G}.
    let (runner, spell) = vein_board(2, 2, 0, 1);
    assert!(offers_cast(&runner, spell));
}

/// Synthetic (labelled) land with a renewable and a self-sacrificing mana
/// ability of different colors, the Crystal Vein shape the engine's mana
/// check already uses.
const GROVE_VEIN: &str = "{T}: Add {C}.\n{T}, Sacrifice this land: Add {G}.";

#[test]
fn required_sacrifice_land_with_two_mana_abilities_cannot_pay_twice() {
    // CR 118.10: the only land is kept for the sacrifice, so it can tap for
    // {C} but not sacrifice itself for {G}: {G} floating + {C} can't pay a
    // (synthetic, labelled) {G}{G} land-sacrificing spell.
    use engine::types::mana::{ManaType, ManaUnit};
    let board = |floating: usize| {
        let mut scenario = GameScenario::new();
        scenario.at_phase(Phase::PreCombatMain);
        let spell = scenario
            .add_spell_to_hand_from_oracle(P0, "Harrow", true, HARROW)
            .with_mana_cost(ManaCost::Cost {
                shards: vec![ManaCostShard::Green, ManaCostShard::Green],
                generic: 0,
            })
            .id();
        scenario.add_land_from_oracle(P0, "Grove Vein", GROVE_VEIN);
        scenario.with_mana_pool(
            P0,
            (0..floating)
                .map(|_| ManaUnit::new(ManaType::Green, ObjectId(0), false, vec![]))
                .collect(),
        );
        scenario.with_library_top(P0, &["Forest"; 2]);
        (scenario.build(), spell)
    };
    let (runner, spell) = board(1);
    assert!(!offers_cast(&runner, spell));
    let (runner, spell) = board(2);
    assert!(offers_cast(&runner, spell));
}
