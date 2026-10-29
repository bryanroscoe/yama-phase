//! CR 608.2c + CR 608.2d: a transient "protection / hexproof from the chosen
//! color" grant uses the color answered while its own resolution is applied,
//! whether or not that answer persists on the source. CR 607.2d: with no answer
//! in this resolution, it reads the source's linked choice. CR 608.2h: each
//! grant latches its color once, when it is applied.

use std::collections::BTreeSet;

use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::types::ability::{
    AbilityDefinition, AbilityKind, ChoiceType, ChosenAttribute, ContinuousModification, Duration,
    Effect, StaticDefinition, TargetFilter, TargetSelectionMode,
};
use engine::types::actions::GameAction;
use engine::types::game_state::{NamedChoiceSourceBinding, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::keywords::{HexproofFilter, Keyword, ProtectionTarget};
use engine::types::mana::{ManaColor, ManaCost, ManaCostShard, ManaType, ManaUnit};
use engine::types::phase::Phase;
use engine::types::zones::Zone;

/// CR 105.4 + CR 608.2c + CR 608.2d: Brave the Elements' answer governs its
/// next instruction. CR 611.2c + CR 702.16a: only white creatures its caster
/// controls gain protection from that color.
#[test]
fn brave_the_elements_nonpersisting_choice_grants_actual_protection() {
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
    let brave = scenario
        .add_spell_to_hand_from_oracle(
            P0,
            "Brave the Elements",
            true,
            "Choose a color. White creatures you control gain protection from the chosen color until end of turn.",
        )
        .with_mana_cost(ManaCost::Cost {
            generic: 0,
            shards: vec![ManaCostShard::White],
        })
        .id();
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
    assert_eq!(*choice_type, ChoiceType::color());
    assert!(source.is_none());
    assert_eq!(
        options.iter().map(String::as_str).collect::<BTreeSet<_>>(),
        BTreeSet::from(["White", "Blue", "Black", "Red", "Green"])
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
        "the white creature Brave's caster controls gains protection from white"
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

/// "~ gains protection from the chosen color and hexproof from the chosen
/// color until end of turn."
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

/// A creature with `ability` as its only activated ability. With `linked`, it
/// already has a linked Blue color choice (CR 607.2d) from before this
/// resolution.
fn fixture(ability: AbilityDefinition, linked: bool) -> (GameRunner, ObjectId) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let source = scenario
        .add_creature(P0, "Color grant source", 2, 2)
        .with_ability_definition(ability)
        .id();
    let mut runner = scenario.build();
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
            "expected a named-choice prompt, got {:?}",
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
        panic!("answer must go to a live named-choice prompt");
    };
    assert!(options.iter().any(|option| option == choice));
    runner
        .act(GameAction::ChooseOption {
            choice: choice.to_string(),
        })
        .unwrap();
}

/// The grants installed since `baseline`, in order, carry exactly `colors`
/// (`None` = the unresolved chosen-color payload), and every concrete grant
/// reaches the source's keywords.
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
    assert_eq!(effects.len(), colors.len(), "every grant is registered");
    let mut ids = BTreeSet::new();
    for (effect, color) in effects.iter().zip(colors) {
        assert_eq!(effect.source_id, source);
        assert!(ids.insert(effect.id), "each grant has its own id");
        assert_eq!(effect.affected, TargetFilter::SpecificObject { id: source });
        assert_eq!(effect.modifications, modifications(*color));
        if let Some(color) = color {
            for keyword in keywords(Some(*color)) {
                assert!(
                    runner.state().objects[&source].keywords.contains(&keyword),
                    "a concrete grant reaches the recipient: {keyword:?}"
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

/// CR 608.2c + CR 608.2d: a later non-color answer doesn't replace the color
/// used by later instructions. CR 607.2d: the source's linked Blue stays
/// independent of this resolution's Red.
fn binding_case(binding: Option<NamedChoiceSourceBinding>) {
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
        // A plain color prompt never carries this binding; rebind the live
        // prompt (keeping its context and continuation) to reach that arm.
        if let WaitingFor::NamedChoice {
            source: Some(prompt_source),
            ..
        } = &mut runner.state_mut().waiting_for
        {
            assert!(prompt_source.has_matching_context());
            prompt_source.binding = NamedChoiceSourceBinding::ResolutionContext;
        } else {
            panic!("expected an exact-object prompt source");
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
fn source_less_color_answer_survives_a_later_noncolor_answer() {
    binding_case(None);
}

#[test]
fn exact_object_color_answer_preserves_the_linked_choice() {
    binding_case(Some(NamedChoiceSourceBinding::ExactObjectAndResolution));
}

#[test]
fn resolution_context_color_answer_preserves_the_linked_choice() {
    binding_case(Some(NamedChoiceSourceBinding::ResolutionContext));
}

/// CR 608.2d: a random single-color answer binds like an interactive one.
#[test]
fn random_nonpersisting_color_answer_is_granted() {
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

/// CR 608.2h: both keyword payloads latch once and keep their color across a
/// later answer in the same resolution.
#[test]
fn each_grant_latches_its_own_nonpersisting_color() {
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

/// CR 607.2d: with no answer in this resolution, the grant reads the source's
/// linked choice.
#[test]
fn grant_without_an_answer_uses_the_linked_choice() {
    let (mut runner, source) = fixture(grant(), true);
    let baseline = runner.state().transient_continuous_effects.len();
    runner.activate(source, 0).resolve();
    assert_payloads(&runner, source, baseline, &[Some(ManaColor::Blue)]);
    assert_complete(&runner);
}

/// CR 609.3: with no answer at all, the grant keeps its unresolved payload; a
/// later grant in the same resolution still latches the real answer.
#[test]
fn grant_with_no_answer_keeps_the_unresolved_payload() {
    let (mut runner, source) = fixture(grant().sub_ability(color_choice(false, grant())), false);
    let baseline = runner.state().transient_continuous_effects.len();
    runner.activate(source, 0).resolve();
    assert_prompt(&runner, ChoiceType::color(), None);
    assert_payloads(&runner, source, baseline, &[None]);
    answer(&mut runner, "White");
    assert_payloads(&runner, source, baseline, &[None, Some(ManaColor::White)]);
    assert_complete(&runner);
}
