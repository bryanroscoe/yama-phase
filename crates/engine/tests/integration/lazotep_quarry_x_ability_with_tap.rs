//! CR 107.3 + CR 601.2h + CR 602.2b: Lazotep Quarry's "{X}{2}, {T}, Sacrifice a
//! Desert: Exile target creature card with mana value X from your graveyard.
//! Create a token that's a copy of it, except it's a 4/4 black Zombie." Before the
//! fix an {X} activation whose cost also taps the source was refused outright:
//! after the X mana, only sacrifice legs were accepted in the remaining cost, and
//! the {T} leg made it "unhandled". The Quarry is itself a Desert, so this also
//! sacrifices the Quarry to its own ability (tapped first).

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
const LAZOTEP_QUARRY: &str = "{T}: Add {C}.\n{T}, Sacrifice a creature: Add one mana of any color.\n{X}{2}, {T}, Sacrifice a Desert: Exile target creature card with mana value X from your graveyard. Create a token that's a copy of it, except it's a 4/4 black Zombie. Activate only as a sorcery.";

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
fn lazotep_quarry_x_ability_with_tap_and_desert_sacrifice() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let quarry = {
        let mut b = scenario.add_land_from_oracle(P0, "Lazotep Quarry", LAZOTEP_QUARRY);
        b.with_subtypes(vec!["Desert"]);
        b.id()
    };
    for _ in 0..4 {
        scenario.add_basic_land(P0, ManaColor::Black);
    }
    let bears = scenario
        .add_creature_to_graveyard(P0, "Grizzly Bears", 2, 2)
        .with_mana_cost(engine::types::mana::ManaCost::generic(2))
        .id();
    let mut runner = scenario.build();

    runner
        .act(GameAction::ActivateAbility {
            source_id: quarry,
            ability_index: 2,
            payment_mode: engine::types::game_state::CastPaymentMode::Auto,
        })
        .expect("the {X} ability is offered and accepted");
    drive(&mut runner, quarry, bears, 2);

    assert_eq!(runner.state().objects[&quarry].zone, Zone::Graveyard);
    assert_eq!(runner.state().objects[&bears].zone, Zone::Exile);
    let zombie = runner
        .state()
        .battlefield
        .iter()
        .copied()
        .find(|id| runner.state().objects[id].is_token)
        .expect("a token copy of the Bears");
    let token = &runner.state().objects[&zombie];
    assert_eq!((token.power, token.toughness), (Some(4), Some(4)));
}
