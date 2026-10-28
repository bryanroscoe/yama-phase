//! Named counters whose meaning lives in a static on the card that places them
//! (CR 122.1 + CR 613.1). Omo, Queen of Vesuva puts an everything counter on
//! "up to one target land and up to one target creature", and her statics make
//! each land with one every land type and each nonland creature with one every
//! creature type: a pure upgrade for the bearer, so the AI should place the
//! counters on its own permanents.

use engine::game::combat::AttackTarget;
use engine::game::scenario::{GameScenario, P0, P1};
use engine::types::ability::{Comparator, TypedFilter};
use engine::types::ability::{FilterProp, QuantityExpr, StaticDefinition, TargetFilter, TargetRef};
use engine::types::actions::GameAction;
use engine::types::counter::{CounterMatch, CounterType};
use engine::types::game_state::{GameState, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::mana::ManaColor;
use engine::types::phase::Phase;
use engine::types::statics::StaticMode;
use rand::rngs::SmallRng;
use rand::SeedableRng;

use crate::config::{create_config, AiConfig, AiDifficulty, Platform};
use crate::policies::anti_self_harm::AntiSelfHarmPolicy;
use crate::policies::context::{PolicyContext, SearchDepth};
use crate::policies::effect_classify::{counter_polarity_for_source, EffectPolarity};
use crate::search::choose_action;

const OMO: &str = "Whenever Omo enters or attacks, put an everything counter on each of up to one target land and up to one target creature.\nEach land with an everything counter on it is every land type in addition to its other types.\nEach nonland creature with an everything counter on it is every creature type.";

fn everything() -> CounterType {
    CounterType::Generic("everything".to_string())
}

#[test]
fn omo_everything_counter_reads_as_beneficial_to_its_bearer() {
    let mut scenario = GameScenario::new();
    let omo = scenario
        .add_creature_from_oracle(P0, "Omo, Queen of Vesuva", 1, 5, OMO)
        .id();
    let runner = scenario.build();
    let omo = &runner.state().objects[&omo];

    assert_eq!(
        counter_polarity_for_source(&everything(), Some(omo)),
        EffectPolarity::Beneficial,
        "Omo's statics define the everything counter as an upgrade for its bearer"
    );
    // Without the placing card the name alone carries no polarity.
    assert_eq!(
        counter_polarity_for_source(&everything(), None),
        EffectPolarity::Contextual
    );
    // A counter Omo's statics don't mention stays card-specific.
    assert_eq!(
        counter_polarity_for_source(&CounterType::Generic("quest".to_string()), Some(omo)),
        EffectPolarity::Contextual
    );
}

#[test]
fn named_counter_with_a_restricting_static_reads_as_harmful() {
    // "Each creature with a doom counter on it can't block." The same rule read
    // the other way: the static restricts the bearer, so the counter harms it.
    let mut scenario = GameScenario::new();
    let source = scenario
        .add_creature(P0, "Doom Herald", 2, 2)
        .with_static_definition(StaticDefinition::new(StaticMode::CantBlock).affected(
            TargetFilter::Typed(
                TypedFilter::creature().properties(vec![FilterProp::Counters {
                    counters: CounterMatch::OfType(CounterType::Generic("doom".to_string())),
                    comparator: Comparator::GE,
                    count: QuantityExpr::Fixed { value: 1 },
                }]),
            ),
        ))
        .id();
    let runner = scenario.build();
    assert_eq!(
        counter_polarity_for_source(
            &CounterType::Generic("doom".to_string()),
            Some(&runner.state().objects[&source])
        ),
        EffectPolarity::Harmful
    );
}

struct OmoBoard {
    state: GameState,
    own: Vec<ObjectId>,
    opponents: Vec<ObjectId>,
}

/// P0 attacks with Omo, so her attack trigger asks P0 for "up to one target
/// land". P0 and P1 each have two lands and a creature besides Omo.
fn omo_attack_trigger() -> OmoBoard {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let omo = scenario
        .add_creature_from_oracle(P0, "Omo, Queen of Vesuva", 1, 5, OMO)
        .id();
    let own_land_a = scenario.add_basic_land(P0, ManaColor::Green);
    let own_land_b = scenario.add_basic_land(P0, ManaColor::Blue);
    let own_bear = scenario.add_creature(P0, "Own Bear", 2, 2).id();
    let opp_land_a = scenario.add_basic_land(P1, ManaColor::Green);
    let opp_land_b = scenario.add_basic_land(P1, ManaColor::Black);
    let opp_bear = scenario.add_creature(P1, "Opponent Bear", 2, 2).id();
    let mut runner = scenario.build();
    runner.advance_to_combat();
    runner
        .declare_attackers(&[(omo, AttackTarget::Player(P1))])
        .expect("Omo can attack");
    assert!(
        matches!(
            runner.state().waiting_for,
            WaitingFor::TriggerTargetSelection { .. }
        ),
        "Omo's attack trigger asks for targets, got {:?}",
        runner.state().waiting_for
    );
    OmoBoard {
        state: runner.state().clone(),
        own: vec![omo, own_land_a, own_land_b, own_bear],
        opponents: vec![opp_land_a, opp_land_b, opp_bear],
    }
}

fn anti_self_harm_score(state: &GameState, target: ObjectId) -> f64 {
    let config = AiConfig::default();
    let action = GameAction::ChooseTarget {
        target: Some(TargetRef::Object(target)),
    };
    let decision = engine::ai_support::build_decision_context_for_semantic_owner(state, P0);
    let candidate = decision
        .candidates
        .iter()
        .find(|candidate| candidate.action == action)
        .expect("the target is a legal choice");
    let ctx = PolicyContext {
        state,
        decision: &decision,
        candidate,
        ai_player: P0,
        config: &config,
        context: &crate::context::AiContext::empty(&config.weights),
        cast_facts: None,
        search_depth: SearchDepth::Root,
    };
    AntiSelfHarmPolicy.score(&ctx)
}

#[test]
fn omo_trigger_scores_own_lands_above_opponents_lands() {
    let board = omo_attack_trigger();
    let own_land = board.own[1];
    let opp_land = board.opponents[0];
    let own = anti_self_harm_score(&board.state, own_land);
    let opp = anti_self_harm_score(&board.state, opp_land);
    assert!(
        own > opp,
        "an everything counter belongs on the AI's own land: own {own}, opponent {opp}"
    );
}

#[test]
fn easy_ai_puts_omos_counters_on_its_own_permanents() {
    // The Easy AI samples from a softmax over its scores, so this counts over
    // seeds.
    let config = create_config(AiDifficulty::Easy, Platform::Native);
    let (mut own, mut opponents, mut none) = (0, 0, 0);
    for seed in 0..40u64 {
        let board = omo_attack_trigger();
        let mut state = board.state;
        let mut rng = SmallRng::seed_from_u64(seed);
        for _ in 0..4 {
            if !matches!(state.waiting_for, WaitingFor::TriggerTargetSelection { .. }) {
                break;
            }
            let action = choose_action(&state, P0, &config, &mut rng).expect("AI answers");
            match &action {
                GameAction::ChooseTarget {
                    target: Some(TargetRef::Object(id)),
                } if board.own.contains(id) => own += 1,
                GameAction::ChooseTarget {
                    target: Some(TargetRef::Object(id)),
                } if board.opponents.contains(id) => opponents += 1,
                GameAction::ChooseTarget { target: None } => none += 1,
                other => panic!("unexpected answer to Omo's trigger: {other:?}"),
            }
            engine::game::apply_as_current(&mut state, action).expect("legal target");
        }
    }
    assert!(
        own >= 3 * (opponents + none),
        "Omo's counters should mostly go on the AI's own permanents: own {own}, opponents {opponents}, none {none}"
    );
}
