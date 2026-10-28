//! Bounded production-path color-choice regressions. Synthetic typed chains test
//! existing building blocks; only the explicitly marked routing case alters a prompt.

use engine::game::scenario::{GameRunner, GameScenario, P0};
use engine::types::ability::{
    AbilityDefinition, AbilityKind, ChoiceType, ChosenAttribute, ContinuousModification, Duration,
    Effect, StaticDefinition, TargetFilter, TargetSelectionMode,
};
use engine::types::actions::GameAction;
use engine::types::game_state::{NamedChoiceSourceBinding, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::keywords::{HexproofFilter, Keyword, ProtectionTarget};
use engine::types::mana::ManaColor;
use engine::types::phase::Phase;
use engine::types::zones::Zone;

#[path = "../integration/activation_cost_work_census.rs"]
mod activation_cost_work_census;
#[path = "../integration/chosen_color_rechoose_same_source.rs"]
mod chosen_color_rechoose_same_source;
#[path = "../integration/delayed_trigger_occurrence_fanout.rs"]
mod delayed_trigger_occurrence_fanout;
#[path = "../integration/named_choice_legal_actions.rs"]
mod named_choice_legal_actions;
#[path = "../integration/no_top_level_test_binaries.rs"]
mod no_top_level_test_binaries;
#[path = "../../src/source_census.rs"]
mod source_census;
#[path = "../integration/waiting_for_actor_authority_census.rs"]
mod waiting_for_actor_authority_census;

/// CR 105.4 + CR 608.2c + CR 608.2d: Brave's answer governs its next
/// instruction. CR 611.2c + CR 702.16a: only controlled white creatures
/// receive protection from that color.
#[test]
fn brave_the_elements_nonpersisting_choice_grants_actual_protection() {
    use engine::game::scenario::{GameScenario, P0, P1};
    use engine::types::actions::GameAction;
    use engine::types::game_state::WaitingFor;
    use engine::types::keywords::ProtectionTarget;
    use engine::types::mana::{ManaColor, ManaCost, ManaCostShard, ManaType, ManaUnit};
    use engine::types::phase::Phase;

    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let white = scenario
        .add_creature(P0, "White creature", 2, 2)
        .with_color(vec![ManaColor::White])
        .id();
    let green = scenario
        .add_creature(P0, "Green creature", 2, 2)
        .with_color(vec![ManaColor::Green])
        .id();
    let opponent = scenario
        .add_creature(P1, "Other white creature", 2, 2)
        .with_color(vec![ManaColor::White])
        .id();
    let brave = scenario.add_spell_to_hand_from_oracle(
            P0, "Brave the Elements", true,
            "Choose a color. White creatures you control gain protection from the chosen color until end of turn.",
        ).with_mana_cost(ManaCost::Cost { generic: 0, shards: vec![ManaCostShard::White] }).id();
    scenario.with_mana_pool(
        P0,
        vec![ManaUnit::new(ManaType::White, brave, false, vec![])],
    );
    let mut runner = scenario.build();
    runner.state_mut().active_player = P0;
    runner.state_mut().priority_player = P0;
    runner.state_mut().waiting_for = WaitingFor::Priority { player: P0 };
    let outcome = runner.cast(brave).resolve();
    let WaitingFor::NamedChoice {
        player,
        choice_type,
        source,
        options,
        ..
    } = outcome.final_waiting_for()
    else {
        panic!("Brave must reach its color prompt");
    };
    assert_eq!(*player, P0);
    assert_eq!(*choice_type, engine::types::ability::ChoiceType::color());
    assert!(source.is_none());
    assert_eq!(
        options
            .iter()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from(["White", "Blue", "Black", "Red", "Green"])
    );
    runner
        .act(GameAction::ChooseOption {
            choice: "White".to_string(),
        })
        .unwrap();
    let protection = Keyword::Protection(ProtectionTarget::Color(ManaColor::White));
    assert!(
        runner.state().objects[&white]
            .keywords
            .contains(&protection),
        "Brave's actual White protection must be granted independently of AI validation"
    );
    assert!(!runner.state().objects[&green]
        .keywords
        .contains(&protection));
    assert!(!runner.state().objects[&opponent]
        .keywords
        .contains(&protection));
    assert_eq!(runner.state().objects[&brave].zone, Zone::Graveyard);
    assert!(matches!(
        runner.state().waiting_for,
        WaitingFor::Priority { .. }
    ));
}

fn keywords(color: Option<ManaColor>) -> Vec<Keyword> {
    vec![
        Keyword::Protection(color.map_or(ProtectionTarget::ChosenColor, ProtectionTarget::Color)),
        Keyword::HexproofFrom(color.map_or(HexproofFilter::ChosenColor, HexproofFilter::Color)),
    ]
}

fn modifications(color: Option<ManaColor>) -> Vec<ContinuousModification> {
    keywords(color)
        .into_iter()
        .map(|keyword| ContinuousModification::AddKeyword { keyword })
        .collect()
}

fn grant() -> AbilityDefinition {
    AbilityDefinition::new(
        AbilityKind::Activated,
        Effect::GenericEffect {
            static_abilities: vec![StaticDefinition::continuous()
                .affected(TargetFilter::SelfRef)
                .modifications(modifications(None))],
            duration: Some(Duration::UntilEndOfTurn),
            target: None,
            end_cost: None,
        },
    )
    .duration(Duration::UntilEndOfTurn)
}

fn choose(
    choice_type: ChoiceType,
    persist: bool,
    selection: TargetSelectionMode,
    next: AbilityDefinition,
) -> AbilityDefinition {
    AbilityDefinition::new(
        AbilityKind::Activated,
        Effect::Choose {
            choice_type,
            persist,
            selection,
        },
    )
    .sub_ability(next)
}

fn color_choice(persist: bool, next: AbilityDefinition) -> AbilityDefinition {
    choose(
        ChoiceType::color(),
        persist,
        TargetSelectionMode::Chosen,
        next,
    )
}

fn fixture(ability: AbilityDefinition, linked: bool) -> (GameRunner, ObjectId) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let source = scenario
        .add_creature(P0, "Typed color-chain fixture", 2, 2)
        .with_ability_definition(ability)
        .id();
    let mut runner = scenario.build();
    // Existing linked supplier answer is fixture input, never a resolution slot.
    if linked {
        runner
            .state_mut()
            .objects
            .get_mut(&source)
            .unwrap()
            .chosen_attributes
            .push(ChosenAttribute::Color(ManaColor::Blue));
    }
    (runner, source)
}

fn assert_prompt(
    runner: &GameRunner,
    expected: ChoiceType,
    binding: Option<NamedChoiceSourceBinding>,
) {
    let WaitingFor::NamedChoice {
        player,
        choice_type,
        options,
        source,
        ..
    } = &runner.state().waiting_for
    else {
        panic!(
            "expected a real named-choice continuation, got {:?}",
            runner.state().waiting_for
        );
    };
    assert_eq!(*player, P0);
    assert_eq!(*choice_type, expected);
    assert!(!options.is_empty());
    assert_eq!(source.as_ref().map(|source| source.binding), binding);
    if let Some(source) = source {
        assert!(source.has_matching_context());
    }
}

fn answer(runner: &mut GameRunner, choice: &str) {
    let WaitingFor::NamedChoice { options, .. } = &runner.state().waiting_for else {
        panic!("answer must submit to a live named-choice prompt");
    };
    assert!(options.iter().any(|option| option == choice));
    runner
        .act(GameAction::ChooseOption {
            choice: choice.to_string(),
        })
        .unwrap();
}

fn assert_payloads(
    runner: &GameRunner,
    source: ObjectId,
    baseline: usize,
    colors: &[Option<ManaColor>],
) {
    let effects: Vec<_> = runner
        .state()
        .transient_continuous_effects
        .iter()
        .skip(baseline)
        .collect();
    assert_eq!(
        effects.len(),
        colors.len(),
        "the chain must actually register every grant"
    );
    let mut ids = std::collections::BTreeSet::new();
    for (effect, color) in effects.iter().zip(colors) {
        assert_eq!(effect.source_id, source);
        assert!(
            ids.insert(effect.id),
            "each installation must have a distinct id"
        );
        assert_eq!(effect.affected, TargetFilter::SpecificObject { id: source });
        assert_eq!(effect.modifications, modifications(*color));
        if let Some(color) = color {
            for keyword in keywords(Some(*color)) {
                assert!(
                    runner.state().objects[&source].keywords.contains(&keyword),
                    "installed concrete grant must reach recipient layers: {keyword:?}"
                );
            }
        }
    }
}

fn assert_complete(runner: &GameRunner) {
    assert!(runner.state().stack.is_empty());
    assert!(matches!(
        runner.state().waiting_for,
        WaitingFor::Priority { .. }
    ));
}

fn binding_case(binding: Option<NamedChoiceSourceBinding>) {
    // CR 608.2c/d: a noncolor answer cannot replace the color used by later instructions.
    // CR 607.2d: the oldest linked Blue remains independent of local Red.
    let label = ChoiceType::Labeled {
        options: vec!["Other".to_string()],
    };
    let chain = grant().sub_ability(color_choice(
        binding.is_some(),
        choose(label.clone(), false, TargetSelectionMode::Chosen, grant()),
    ));
    let (mut runner, source) = fixture(chain, true);
    let baseline = runner.state().transient_continuous_effects.len();
    runner.activate(source, 0).resolve();
    assert_prompt(
        &runner,
        ChoiceType::color(),
        binding.map(|_| NamedChoiceSourceBinding::ExactObjectAndResolution),
    );
    assert_payloads(&runner, source, baseline, &[Some(ManaColor::Blue)]);
    if binding == Some(NamedChoiceSourceBinding::ResolutionContext) {
        // ROUTING ONLY: ordinary Color does not naturally emit this binding.
        // Preserve production-created context and continuation; mutate only binding.
        if let WaitingFor::NamedChoice {
            source: Some(prompt_source),
            ..
        } = &mut runner.state_mut().waiting_for
        {
            assert!(prompt_source.has_matching_context());
            prompt_source.binding = NamedChoiceSourceBinding::ResolutionContext;
        } else {
            panic!("production-created exact source required");
        }
        assert_prompt(&runner, ChoiceType::color(), binding);
    }
    answer(&mut runner, "Red");
    assert_prompt(&runner, label, None);
    assert_eq!(
        runner.state().named_color_this_resolution,
        Some(ManaColor::Red)
    );
    let exact = binding == Some(NamedChoiceSourceBinding::ExactObjectAndResolution);
    // Output of the common binder distinguishes the routing arm from exact
    // persistence; successful action plus this write proves no earlier guard won.
    assert_eq!(
        runner.state().chosen_color_this_resolution,
        exact.then_some(ManaColor::Red)
    );
    let stored: Vec<_> = runner.state().objects[&source]
        .chosen_attributes
        .iter()
        .filter_map(|attribute| match attribute {
            ChosenAttribute::Color(color) => Some(*color),
            _ => None,
        })
        .collect();
    assert_eq!(
        stored,
        if exact {
            vec![ManaColor::Blue, ManaColor::Red]
        } else {
            vec![ManaColor::Blue]
        }
    );
    assert_eq!(
        runner.state().objects[&source].chosen_color(),
        Some(ManaColor::Blue)
    );
    answer(&mut runner, "Other");
    assert_payloads(
        &runner,
        source,
        baseline,
        &[Some(ManaColor::Blue), Some(ManaColor::Red)],
    );
    assert_eq!(
        runner.state().objects[&source].chosen_color(),
        Some(ManaColor::Blue)
    );
    assert_complete(&runner);
}

#[test]
fn resolution_chosen_color_source_less_survives_noncolor_answer() {
    binding_case(None);
}
#[test]
fn resolution_chosen_color_exact_binding_preserves_linked_choice() {
    binding_case(Some(NamedChoiceSourceBinding::ExactObjectAndResolution));
}
#[test]
fn resolution_chosen_color_resolution_context_routing_preserves_linked_choice() {
    binding_case(Some(NamedChoiceSourceBinding::ResolutionContext));
}

/// CR 608.2d: random and interactive single-color answers share the binder.
#[test]
fn resolution_chosen_color_reads_random_nonpersisting_answer() {
    let choice_type = ChoiceType::Color {
        excluded: ManaColor::ALL
            .into_iter()
            .filter(|color| *color != ManaColor::White)
            .collect(),
    };
    let (mut runner, source) = fixture(
        choose(choice_type, false, TargetSelectionMode::Random, grant()),
        false,
    );
    let baseline = runner.state().transient_continuous_effects.len();
    let outcome = runner.activate(source, 0).resolve();
    assert!(matches!(
        outcome.final_waiting_for(),
        WaitingFor::Priority { .. }
    ));
    assert_payloads(&runner, source, baseline, &[Some(ManaColor::White)]);
    assert_complete(&runner);
}

/// CR 608.2h: both installed keyword payloads latch once, across later answers.
#[test]
fn snapshot_latches_nonpersisting_colors_for_both_keyword_arms() {
    let chain = color_choice(false, grant().sub_ability(color_choice(false, grant())));
    let (mut runner, source) = fixture(chain, true);
    let baseline = runner.state().transient_continuous_effects.len();
    runner.activate(source, 0).resolve();
    assert_prompt(&runner, ChoiceType::color(), None);
    answer(&mut runner, "White");
    assert_prompt(&runner, ChoiceType::color(), None);
    assert_payloads(&runner, source, baseline, &[Some(ManaColor::White)]);
    answer(&mut runner, "Red");
    assert_payloads(
        &runner,
        source,
        baseline,
        &[Some(ManaColor::White), Some(ManaColor::Red)],
    );
    assert_eq!(
        runner.state().objects[&source].chosen_color(),
        Some(ManaColor::Blue)
    );
    assert_complete(&runner);
}

/// CR 607.2d: absent a new answer, both grants read their source's linked answer.
#[test]
fn snapshot_without_fresh_answer_uses_linked_blue() {
    let (mut runner, source) = fixture(grant(), true);
    let baseline = runner.state().transient_continuous_effects.len();
    runner.activate(source, 0).resolve();
    assert_payloads(&runner, source, baseline, &[Some(ManaColor::Blue)]);
    assert_complete(&runner);
}

/// CR 609.3: a genuinely absent answer leaves the exact unresolved payloads;
/// the later White grant proves registration and both snapshot arms are reachable.
#[test]
fn snapshot_missing_answer_preserves_payload_then_latches_real_answer() {
    let (mut runner, source) = fixture(grant().sub_ability(color_choice(false, grant())), false);
    let baseline = runner.state().transient_continuous_effects.len();
    runner.activate(source, 0).resolve();
    assert_prompt(&runner, ChoiceType::color(), None);
    assert_payloads(&runner, source, baseline, &[None]);
    answer(&mut runner, "White");
    assert_payloads(&runner, source, baseline, &[None, Some(ManaColor::White)]);
    assert_complete(&runner);
}
