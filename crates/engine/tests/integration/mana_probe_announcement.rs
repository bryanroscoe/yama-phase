//! Reduced public-action witnesses for Yama's frozen multiplayer acceptance cases.
use super::support::shared_card_db;
use engine::ai_support::AiDecisionContract;
use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::game::scenario_db::GameScenarioDbExt;
use engine::types::actions::GameAction;
use engine::types::game_state::{CastPaymentMode, ManaChoice, WaitingFor};
use engine::types::mana::{ManaColor, ManaCost, ManaCostShard, ManaType};
use engine::types::phase::Phase;
use engine::types::zones::Zone;

fn filter_board() -> GameScenario {
    let db = shared_card_db().expect("committed card fixture required");
    let mut s = GameScenario::new();
    s.at_phase(Phase::PreCombatMain);
    s.add_real_card(P0, "Fetid Heath", Zone::Battlefield, db);
    s.add_real_card(P0, "Darkwater Catacombs", Zone::Battlefield, db);
    s.add_basic_land(P0, ManaColor::Black);
    s
}

#[test]
fn adventure_filter_payment_has_engine_issued_faces() {
    let db = shared_card_db().unwrap();
    let mut s = filter_board();
    let rider = s.add_real_card(P0, "Murderous Rider", Zone::Hand, db);
    let target = s.add_vanilla(P1, 2, 2);
    let mut r = s.build();
    let card_id = r.state().objects[&rider].card_id;
    r.act(GameAction::CastSpell {
        object_id: rider,
        card_id,
        targets: vec![],
        payment_mode: CastPaymentMode::Auto,
    })
    .unwrap();
    assert!(matches!(
        r.state().waiting_for,
        WaitingFor::CastOffer { player: P0, .. }
    ));
    // CR 715.3a + 601.2g: both {1}{B}{B} faces are payable via the filter lands.
    let contract = AiDecisionContract::issue(r.state(), P0);
    for creature in [true, false] {
        let action = GameAction::ChooseAdventureFace { creature };
        assert!(
            contract.contains_action(r.state(), &action),
            "missing face {creature}"
        );
    }
    r.act(GameAction::ChooseAdventureFace { creature: false })
        .unwrap();
    if matches!(r.state().waiting_for, WaitingFor::TargetSelection { .. }) {
        r.choose_first_legal_target().unwrap();
    }
    pay_through_filter_lands(&mut r);
    r.resolve_top();
    assert_eq!(r.state().objects[&target].zone, Zone::Graveyard);
    // CR 715.3d: the resolved Adventure is exiled, with its creature still playable.
    assert_eq!(r.state().objects[&rider].zone, Zone::Exile);
    assert_eq!(r.state().players[0].life, 18);
    assert!(r.state().stack.is_empty());
    assert!(matches!(r.state().waiting_for, WaitingFor::Priority { .. }));
}

#[test]
fn targeted_cast_filter_probe_does_not_open_nested_action() {
    let db = shared_card_db().unwrap();
    let mut s = filter_board();
    let spell = s.add_real_card(P0, "Hero's Downfall", Zone::Hand, db);
    let target = s.add_vanilla(P1, 2, 2);
    s.add_vanilla(P1, 3, 3);
    let mut r = s.build();
    // CR 601.2c/g/h: choosing a target precedes mana activation/payment.
    let card_id = r.state().objects[&spell].card_id;
    r.act(GameAction::CastSpell {
        object_id: spell,
        card_id,
        targets: vec![],
        payment_mode: CastPaymentMode::Auto,
    })
    .unwrap();
    assert!(matches!(
        r.state().waiting_for,
        WaitingFor::TargetSelection { .. }
    ));
    r.choose_first_legal_target().unwrap();
    pay_through_filter_lands(&mut r);
    r.resolve_top();
    assert_eq!(r.state().objects[&target].zone, Zone::Graveyard);
    assert_eq!(r.state().objects[&spell].zone, Zone::Graveyard);
    assert!(r.state().stack.is_empty());
    assert!(matches!(r.state().waiting_for, WaitingFor::Priority { .. }));
}

// CR 601.2g/h + 605.3a/b: explicitly take the filter route; the ordinary auto-
// tap heuristic does not schedule it. Every payment/color choice is engine-issued.
fn pay_through_filter_lands(r: &mut GameRunner) {
    assert!(matches!(
        r.state().waiting_for,
        WaitingFor::ManaPayment { player: P0, .. }
    ));
    for name in ["Swamp", "Darkwater Catacombs", "Fetid Heath"] {
        let source = r
            .state()
            .battlefield
            .iter()
            .copied()
            .find(|id| r.state().objects[id].name == name)
            .unwrap();
        if name == "Swamp" {
            let selection =
                engine::game::mana_sources::activatable_mana_source_selections(r.state(), P0)
                    .into_iter()
                    .find(|selection| selection.source.object_id == source)
                    .unwrap();
            r.act(GameAction::TapLandForMana { selection }).unwrap();
        } else {
            r.act(GameAction::ActivateAbility {
                source_id: source,
                ability_index: if name == "Fetid Heath" { 1 } else { 0 },
                payment_mode: CastPaymentMode::Auto,
            })
            .unwrap();
        }
        for _ in 0..4 {
            match r.state().waiting_for.clone() {
                WaitingFor::PayManaAbilityMana { options, .. } => {
                    let payment = options
                        .into_iter()
                        .find(|payment| payment == &vec![ManaType::Black])
                        .expect("black funds the filter");
                    r.act(GameAction::PayManaAbilityMana { payment }).unwrap();
                }
                WaitingFor::ChooseManaColor { .. } => {
                    r.act(GameAction::ChooseManaColor {
                        choice: ManaChoice::Combination(vec![ManaType::Black, ManaType::Black]),
                        count: 1,
                    })
                    .unwrap();
                }
                WaitingFor::ManaPayment { .. } => break,
                other => panic!("unexpected mana continuation {other:?}"),
            }
        }
        assert!(matches!(
            r.state().waiting_for,
            WaitingFor::ManaPayment { .. }
        ));
    }
    r.act(GameAction::PassPriority).unwrap();
    assert!(matches!(r.state().waiting_for, WaitingFor::Priority { .. }));
}

#[test]
fn unpayable_modal_choice_preserves_the_enclosing_announcement() {
    let mut s = filter_board();
    let spell = s.add_spell_to_hand_from_oracle(P0, "Austere Command", false, "Choose two —\n• Destroy all artifacts.\n• Destroy all enchantments.\n• Destroy all creatures with mana value 3 or less.\n• Destroy all creatures with mana value 4 or greater.")
        .with_mana_cost(ManaCost::Cost {shards:vec![ManaCostShard::White,ManaCostShard::White],generic:4}).id();
    let mut r = s.build();
    let card_id = r.state().objects[&spell].card_id;
    r.act(GameAction::CastSpell {
        object_id: spell,
        card_id,
        targets: vec![],
        payment_mode: CastPaymentMode::Auto,
    })
    .unwrap();
    assert!(matches!(
        r.state().waiting_for,
        WaitingFor::ModeChoice { .. }
    ));
    // CR 601.2b/f/h: modes are announced before the six-mana total is paid.
    // Three lands cannot pay it. The speculative mana walk must return a clean
    // refusal, without beginning another action inside this announcement.
    let error = r
        .act(GameAction::SelectModes {
            indices: vec![0, 1],
        })
        .unwrap_err();
    assert!(error.to_string().contains("mana"), "{error}");
    assert!(matches!(
        r.state().waiting_for,
        WaitingFor::ModeChoice { .. }
    ));
    assert!(r
        .state()
        .battlefield
        .iter()
        .all(|id| !r.state().objects[id].tapped));
}

#[test]
fn modal_target_announcement_does_not_nest_action_boundaries() {
    let mut s = filter_board();
    let db = shared_card_db().unwrap();
    s.add_real_card(P1, "Sol Ring", Zone::Battlefield, db);
    let spell=s.add_spell_to_hand_from_oracle(P0,"Hull Breach",false,"Choose one —\n• Destroy target artifact.\n• Destroy target enchantment.\n• Destroy target artifact and target enchantment.")
        .with_mana_cost(ManaCost::Cost {shards:vec![ManaCostShard::Red,ManaCostShard::Green],generic:0}).id();
    let mut r = s.build();
    let card_id = r.state().objects[&spell].card_id;
    r.act(GameAction::CastSpell {
        object_id: spell,
        card_id,
        targets: vec![],
        payment_mode: CastPaymentMode::Auto,
    })
    .unwrap();
    assert!(matches!(
        r.state().waiting_for,
        WaitingFor::ModeChoice { .. }
    ));
    // CR 601.2b/c/g: the one artifact target is announced by choosing the mode,
    // before checking payment. The enclosing spell owns that announcement's events.
    let error = r
        .act(GameAction::SelectModes { indices: vec![0] })
        .unwrap_err();
    assert!(error.to_string().contains("mana"), "{error}");
    assert!(matches!(
        r.state().waiting_for,
        WaitingFor::ModeChoice { .. }
    ));
    assert!(r
        .state()
        .battlefield
        .iter()
        .all(|id| !r.state().objects[id].tapped));
}

#[test]
fn single_target_cast_preserves_announcement_carrier_during_payment_probe() {
    let db = shared_card_db().unwrap();
    let mut s = filter_board();
    let spell = s.add_real_card(P0, "Hero's Downfall", Zone::Hand, db);
    let target = s.add_vanilla(P1, 2, 2);
    let mut r = s.build();
    let card_id = r.state().objects[&spell].card_id;
    r.act(GameAction::CastSpell {
        object_id: spell,
        card_id,
        targets: vec![],
        payment_mode: CastPaymentMode::Auto,
    })
    .unwrap();
    // CR 601.2c: exactly one legal target is announced in this same action.
    pay_through_filter_lands(&mut r);
    r.resolve_top();
    assert_eq!(r.state().objects[&target].zone, Zone::Graveyard);
    assert!(r.state().stack.is_empty());
    assert!(matches!(r.state().waiting_for, WaitingFor::Priority { .. }));
}
