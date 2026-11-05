//! CR 702.157a + CR 601.2b + CR 601.2f-h + CR 118.3: a repeatable optional
//! additional cost (squad) may be paid "any number of times", but each payment
//! raises the total cost that must then be paid in full. A further payment is
//! offered only while the raised total stays payable, and declining further
//! payments never leaves the cast stuck: CR 733.1 reverses a cast whose
//! committed total can't be paid.

use engine::ai_support::legal_actions;
use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::types::actions::GameAction;
use engine::types::game_state::{CastPaymentMode, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::mana::{ManaColor, ManaCost, ManaCostShard};
use engine::types::phase::Phase;
use engine::types::zones::Zone;

/// Powder Ganger (Oracle text verified against Scryfall).
const POWDER_GANGER: &str = "Squad {2} (As an additional cost to cast this spell, you may pay {2} any number of times. When this creature enters, create that many tokens that are copies of it.)\nWhen this creature enters, destroy up to one target artifact.";
/// Securitron Squadron (Oracle text verified against Scryfall).
const SECURITRON: &str = "Squad {3} (As an additional cost to cast this spell, you may pay {3} any number of times. When this creature enters, create that many tokens that are copies of it.)\nVigilance\nWhenever a creature token you control enters, put a +1/+1 counter on it.";
const SOL_RING: &str = "{T}: Add {C}{C}.";

struct Board {
    runner: GameRunner,
    spell: ObjectId,
    lands: Vec<ObjectId>,
    sol_ring: ObjectId,
}

fn powder_ganger_board(mountains: usize) -> Board {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let mut builder = scenario.add_creature_to_hand(P0, "Powder Ganger", 2, 2);
    builder.with_mana_cost(ManaCost::Cost {
        generic: 2,
        shards: vec![ManaCostShard::Red],
    });
    builder.from_oracle_text_with_keywords(&["squad:{2}"], POWDER_GANGER);
    let spell = builder.id();
    let lands = (0..mountains)
        .map(|_| scenario.add_basic_land(P0, ManaColor::Red))
        .collect();
    let sol_ring = scenario
        .add_artifact_from_oracle(P1, "Sol Ring", SOL_RING)
        .id();
    Board {
        runner: scenario.build(),
        spell,
        lands,
        sol_ring,
    }
}

fn begin_cast(runner: &mut GameRunner, spell: ObjectId) {
    let card_id = runner.state().objects[&spell].card_id;
    runner
        .act(GameAction::CastSpell {
            object_id: spell,
            card_id,
            targets: vec![],
            payment_mode: CastPaymentMode::Auto,
        })
        .expect("begin the squad cast");
}

fn pay_offered(runner: &GameRunner) -> bool {
    matches!(
        runner.state().waiting_for,
        WaitingFor::OptionalCostChoice { .. }
    ) && legal_actions(runner.state())
        .iter()
        .any(|action| matches!(action, GameAction::DecideOptionalCost { pay: true }))
}

/// Decline further squad payments if the prompt is still up.
fn stop_paying(runner: &mut GameRunner) {
    if matches!(
        runner.state().waiting_for,
        WaitingFor::OptionalCostChoice { .. }
    ) {
        assert!(
            legal_actions(runner.state())
                .iter()
                .any(|action| matches!(action, GameAction::DecideOptionalCost { pay: false })),
            "declining is always offered"
        );
        runner
            .act(GameAction::DecideOptionalCost { pay: false })
            .expect("declining further squad payments is never refused");
    }
}

/// Resolve everything: choose the first legal target for each trigger, and
/// answer any other decision (trigger order) with its first legal action.
fn resolve_all(runner: &mut GameRunner) {
    for _ in 0..80 {
        match runner.state().waiting_for {
            WaitingFor::TriggerTargetSelection { .. } | WaitingFor::TargetSelection { .. } => {
                runner
                    .choose_first_legal_target()
                    .expect("choose a trigger target");
            }
            WaitingFor::Priority { .. } if runner.state().stack.is_empty() => return,
            WaitingFor::Priority { .. } => {
                runner.act(GameAction::PassPriority).expect("pass priority");
            }
            _ => {
                let action = legal_actions(runner.state())
                    .into_iter()
                    .next()
                    .expect("a decision has a legal answer");
                runner.act(action).expect("answer the decision");
            }
        }
    }
    panic!("resolution did not finish");
}

fn named_on_battlefield(runner: &GameRunner, name: &str) -> usize {
    let state = runner.state();
    state
        .battlefield
        .iter()
        .filter_map(|id| state.objects.get(id))
        .filter(|obj| obj.name == name)
        .count()
}

#[test]
fn squad_offers_only_the_payments_the_total_can_afford() {
    let Board {
        mut runner,
        spell,
        lands,
        sol_ring,
    } = powder_ganger_board(5);
    begin_cast(&mut runner, spell);
    assert!(
        pay_offered(&runner),
        "{{4}}{{R}} is payable with five Mountains"
    );
    runner
        .act(GameAction::DecideOptionalCost { pay: true })
        .expect("first squad payment");
    assert!(
        !pay_offered(&runner),
        "a second payment makes the total {{6}}{{R}}, which five Mountains can't pay"
    );
    stop_paying(&mut runner);
    resolve_all(&mut runner);

    let state = runner.state();
    assert!(lands.iter().all(|land| state.objects[land].tapped));
    assert_eq!(state.objects[&spell].zone, Zone::Battlefield);
    assert_eq!(
        named_on_battlefield(&runner, "Powder Ganger"),
        2,
        "the original and one squad token"
    );
    assert_eq!(
        runner.state().objects[&sol_ring].zone,
        Zone::Graveyard,
        "one enters trigger destroyed the only artifact"
    );
}

#[test]
fn squad_with_mana_only_for_the_base_cost_offers_no_payment() {
    let Board {
        mut runner,
        spell,
        lands,
        ..
    } = powder_ganger_board(3);
    begin_cast(&mut runner, spell);
    assert!(
        !pay_offered(&runner),
        "{{4}}{{R}} isn't payable with three Mountains"
    );
    stop_paying(&mut runner);
    resolve_all(&mut runner);
    let state = runner.state();
    assert!(lands.iter().all(|land| state.objects[land].tapped));
    assert_eq!(state.objects[&spell].zone, Zone::Battlefield);
    assert_eq!(named_on_battlefield(&runner, "Powder Ganger"), 1);
}

#[test]
fn declining_after_an_unaffordable_commitment_reverses_the_cast() {
    let Board {
        mut runner,
        spell,
        lands,
        ..
    } = powder_ganger_board(7);
    begin_cast(&mut runner, spell);
    runner
        .act(GameAction::DecideOptionalCost { pay: true })
        .expect("first squad payment");
    assert!(pay_offered(&runner), "seven Mountains could pay {{6}}{{R}}");
    // Mana sources are lost after the payment was announced: the committed
    // {4}{R} is no longer payable with four Mountains.
    {
        let state = runner.state_mut();
        for lost in &lands[..3] {
            state.battlefield.retain(|id| id != lost);
            state.objects.get_mut(lost).unwrap().zone = Zone::Graveyard;
            state.players[0].graveyard.push_back(*lost);
        }
    }
    assert!(
        runner
            .act(GameAction::DecideOptionalCost { pay: true })
            .is_err(),
        "a payment making the total {{6}}{{R}} is refused with four Mountains"
    );
    assert!(matches!(
        runner.state().waiting_for,
        WaitingFor::OptionalCostChoice { .. }
    ));
    let result = runner.act(GameAction::DecideOptionalCost { pay: false });
    assert!(
        result.is_ok(),
        "declining must not be refused with \"Cannot pay mana cost\": {result:?}"
    );
    let state = runner.state();
    assert_eq!(
        state.objects[&spell].zone,
        Zone::Hand,
        "CR 733.1: the cast is reversed"
    );
    assert!(lands[3..].iter().all(|land| !state.objects[land].tapped));
    assert!(matches!(state.waiting_for, WaitingFor::Priority { player } if player == P0));
}

#[test]
fn squad_offers_exactly_as_many_payments_as_the_mana_affords() {
    // Securitron Squadron {1}{W}, squad {3}: eight lands pay {1}{W} plus two squad
    // payments ({7}{W}); a third would need eleven.
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let mut builder = scenario.add_creature_to_hand(P0, "Securitron Squadron", 2, 2);
    builder.with_mana_cost(ManaCost::Cost {
        generic: 1,
        shards: vec![ManaCostShard::White],
    });
    builder.from_oracle_text_with_keywords(&["squad:{3}", "Vigilance"], SECURITRON);
    let spell = builder.id();
    scenario.add_basic_land(P0, ManaColor::White);
    for _ in 0..7 {
        scenario.add_basic_land(P0, ManaColor::Red);
    }
    let mut runner = scenario.build();
    begin_cast(&mut runner, spell);
    let mut payments = 0;
    while pay_offered(&runner) {
        runner
            .act(GameAction::DecideOptionalCost { pay: true })
            .expect("an offered squad payment is accepted");
        payments += 1;
        assert!(payments <= 2, "a third payment can't be afforded");
    }
    assert_eq!(payments, 2);
    stop_paying(&mut runner);
    resolve_all(&mut runner);
    assert_eq!(runner.state().objects[&spell].zone, Zone::Battlefield);
    assert_eq!(named_on_battlefield(&runner, "Securitron Squadron"), 3);
}

/// Train of Thought (Oracle text verified against Scryfall).
const TRAIN_OF_THOUGHT: &str = "Replicate {1}{U} (When you cast this spell, copy it for each time you paid its replicate cost.)\nDraw a card.";

#[test]
fn replicate_offers_only_the_payments_the_total_can_afford() {
    // CR 702.56a: {1}{U} plus one replicate payment is four mana; a second
    // would need six.
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, "Train of Thought", false, TRAIN_OF_THOUGHT)
        .with_mana_cost(ManaCost::Cost {
            generic: 1,
            shards: vec![ManaCostShard::Blue],
        })
        .id();
    let islands: Vec<_> = (0..4)
        .map(|_| scenario.add_basic_land(P0, ManaColor::Blue))
        .collect();
    scenario.with_library_top(P0, &["Wastes"; 4]);
    let mut runner = scenario.build();
    begin_cast(&mut runner, spell);
    assert!(
        pay_offered(&runner),
        "{{3}}{{U}}{{U}} is payable with four Islands"
    );
    runner
        .act(GameAction::DecideOptionalCost { pay: true })
        .expect("first replicate payment");
    assert!(
        !pay_offered(&runner),
        "a second payment would need six Islands"
    );
    stop_paying(&mut runner);
    resolve_all(&mut runner);
    let state = runner.state();
    assert!(islands.iter().all(|land| state.objects[land].tapped));
    assert_eq!(
        state.players[0].hand.len(),
        2,
        "the spell and its one copy each drew a card"
    );
}
