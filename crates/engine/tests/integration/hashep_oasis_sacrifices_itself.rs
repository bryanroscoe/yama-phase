//! CR 601.2h + CR 602.2b: the costs of an activated ability may be paid in any
//! order, so a land whose ability says "{T}, Sacrifice a Desert" may sacrifice
//! itself when it is a Desert: it is tapped first, then sacrificed.
//!
//! Hashep Oasis: "{1}{G}{G}, {T}, Sacrifice a Desert: Target creature gets +3/+3
//! until end of turn." (ruling: the Desert may be Hashep Oasis itself). Before the
//! fix the engine offered the Oasis as the Desert, sacrificed it, and then refused
//! the activation because the {T} source had left the battlefield.

use engine::ai_support::AiDecisionContract;
use engine::game::scenario::{GameRunner, GameScenario, P0};
use engine::types::ability::TargetRef;
use engine::types::actions::GameAction;
use engine::types::game_state::{PayCostKind, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::mana::ManaColor;
use engine::types::phase::Phase;
use engine::types::zones::Zone;

/// Verbatim Oracle text (MTGJSON).
const HASHEP_OASIS: &str = "{T}: Add {C}.\n{T}, Pay 1 life: Add {G}.\n{1}{G}{G}, {T}, Sacrifice a Desert: Target creature gets +3/+3 until end of turn. Activate only as a sorcery.";

/// Answer every decision until the stack is empty: the chosen Desert for the
/// sacrifice, `target` for a target, `x` for X, and the engine's first offer
/// for anything else (paying mana automatically).
fn drive(runner: &mut GameRunner, desert: ObjectId, target: ObjectId, x: u32) {
    for _ in 0..60 {
        let action = match runner.state().waiting_for.clone() {
            WaitingFor::Priority { .. } if runner.state().stack.is_empty() => return,
            WaitingFor::PayCost {
                kind: PayCostKind::Sacrifice,
                ..
            } => GameAction::SelectCards {
                cards: vec![desert],
            },
            WaitingFor::TargetSelection { .. } | WaitingFor::TriggerTargetSelection { .. } => {
                GameAction::ChooseTarget {
                    target: Some(TargetRef::Object(target)),
                }
            }
            WaitingFor::ChooseXValue { .. } => GameAction::ChooseX { value: x },
            _ => {
                let player = runner.state().waiting_for.acting_players()[0];
                AiDecisionContract::issue(runner.state(), player)
                    .candidates
                    .into_iter()
                    .map(|candidate| candidate.action)
                    .find(|action| !matches!(action, GameAction::CancelCast))
                    .expect("the engine offers an answer")
            }
        };
        runner
            .act(action.clone())
            .unwrap_or_else(|e| panic!("{action:?} refused: {e}"));
    }
    panic!("the activation never settled");
}

#[test]
fn hashep_oasis_can_sacrifice_itself_for_its_own_ability() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let oasis = {
        let mut b = scenario.add_land_from_oracle(P0, "Hashep Oasis", HASHEP_OASIS);
        b.with_subtypes(vec!["Desert"]);
        b.id()
    };
    for _ in 0..3 {
        scenario.add_basic_land(P0, ManaColor::Green);
    }
    let bears = scenario.add_creature(P0, "Grizzly Bears", 2, 2).id();
    let mut runner = scenario.build();

    runner
        .act(GameAction::ActivateAbility {
            source_id: oasis,
            ability_index: 2,
        })
        .expect("the pump ability is offered and accepted");
    drive(&mut runner, oasis, bears, 0);

    assert_eq!(
        runner.state().objects[&oasis].zone,
        Zone::Graveyard,
        "the Oasis was tapped and then sacrificed as the Desert"
    );
    let bears_obj = &runner.state().objects[&bears];
    assert_eq!(
        (bears_obj.power, bears_obj.toughness),
        (Some(5), Some(5)),
        "the Bears got +3/+3"
    );
}
