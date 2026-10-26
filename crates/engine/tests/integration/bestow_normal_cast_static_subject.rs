//! A self/enchanted-host static must not create a spell target (CR 604.1).
//! Only casting bestowed creates an Aura target (CR 702.103a + CR 702.103b).

use engine::game::scenario::{GameScenario, P0};
use engine::types::actions::AlternativeCastDecision;
use engine::types::card_type::CoreType;
use engine::types::events::GameEvent;
use engine::types::game_state::{CastingVariant, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::mana::{ManaCost, ManaCostShard, ManaType, ManaUnit};
use engine::types::phase::Phase;
use engine::types::zones::Zone;

// Oracle verified against Scryfall's named-card endpoint on 2026-10-25.
const NOBLE_QUARRY: &str = "Bestow {5}{G} (If you cast this card for its bestow cost, it's an Aura spell with enchant creature. It becomes a creature again if it's not attached.)\n\
    All creatures able to block this creature or enchanted creature do so.\n\
    Enchanted creature gets +1/+1.";

fn add_quarry(scenario: &mut GameScenario) -> ObjectId {
    scenario.at_phase(Phase::PreCombatMain);
    scenario.with_mana_pool(
        P0,
        (0..6)
            .map(|_| ManaUnit::new(ManaType::Green, ObjectId(0), false, vec![]))
            .collect(),
    );
    scenario
        .add_creature_to_hand(P0, "Noble Quarry", 1, 1)
        .as_enchantment()
        .as_creature()
        .with_subtypes(vec!["Unicorn"])
        .from_oracle_text_with_keywords(&["Bestow"], NOBLE_QUARRY)
        .with_mana_cost(ManaCost::Cost {
            shards: vec![ManaCostShard::Green],
            generic: 2,
        })
        .id()
}

#[test]
fn normal_bestow_card_cast_needs_no_target_with_or_without_other_creatures() {
    for has_host in [false, true] {
        let mut scenario = GameScenario::new();
        if has_host {
            scenario.add_creature(P0, "Grizzly Bears", 2, 2);
        }
        let quarry = add_quarry(&mut scenario);
        let mut runner = scenario.build();
        assert!(
            runner.state().objects[&quarry].abilities.is_empty(),
            "printed statics must not become spell effects"
        );
        let outcome = runner
            .cast(quarry)
            .casting_variant(CastingVariant::Normal)
            .alternative_cast(AlternativeCastDecision::Normal)
            .resolve();

        assert!(matches!(
            outcome.final_waiting_for(),
            WaitingFor::Priority { .. }
        ));
        assert!(outcome.state().stack.is_empty());
        assert_eq!(outcome.zone_of(quarry), Zone::Battlefield);
        assert!(outcome.state().objects[&quarry]
            .card_types
            .core_types
            .contains(&CoreType::Creature));
        assert!(outcome.state().objects[&quarry].attached_to.is_none());
        assert_eq!(outcome.power_toughness(quarry), (1, 1));
        assert_eq!(outcome.mana_pool_total(P0), 3, "pay the printed {{2}}{{G}}");
        assert!(
            !outcome.events().iter().any(|event| {
                matches!(event, GameEvent::BecomesTarget { source_id, .. } if *source_id == quarry)
            }),
            "a normal creature cast must announce no targets"
        );
    }
}

#[test]
fn bestowed_cast_still_targets_and_enchants_a_creature() {
    let mut scenario = GameScenario::new();
    let host = scenario.add_creature(P0, "Grizzly Bears", 2, 2).id();
    let quarry = add_quarry(&mut scenario);
    let mut runner = scenario.build();
    let committed = runner
        .cast(quarry)
        .casting_variant(CastingVariant::Bestow)
        .alternative_cast(AlternativeCastDecision::Alternative)
        .target_object(host)
        .commit();

    // CR 702.103b: the Aura form and its target must exist on the stack,
    // before resolution can attach the permanent (CR 608.3c).
    let on_stack = &committed.state().objects[&quarry];
    assert!(on_stack.bestow_form.is_some());
    assert!(!on_stack.card_types.core_types.contains(&CoreType::Creature));
    assert!(on_stack
        .card_types
        .subtypes
        .iter()
        .any(|subtype| subtype == "Aura"));
    let entry = committed
        .state()
        .stack
        .last()
        .expect("bestowed spell on stack");
    assert_eq!(entry.id, quarry);
    let ability = entry.ability().expect("Aura keeps its announced target");
    assert_eq!(
        ability.targets,
        vec![engine::types::ability::TargetRef::Object(host)]
    );
    let outcome = committed.resolve();
    assert!(matches!(
        outcome.final_waiting_for(),
        WaitingFor::Priority { .. }
    ));
    assert!(outcome.state().stack.is_empty());

    assert_eq!(outcome.zone_of(quarry), Zone::Battlefield);
    assert!(!outcome.state().objects[&quarry]
        .card_types
        .core_types
        .contains(&CoreType::Creature));
    assert_eq!(
        outcome.state().objects[&quarry].attached_to,
        Some(engine::game::game_object::AttachTarget::Object(host))
    );
    assert_eq!(outcome.power_toughness(host), (3, 3));
    assert_eq!(outcome.mana_pool_total(P0), 0, "pay bestow's {{5}}{{G}}");
    assert_eq!(
        outcome
            .events()
            .iter()
            .filter(|event| {
                matches!(event, GameEvent::BecomesTarget { source_id, .. } if *source_id == quarry)
            })
            .count(),
        1,
        "the bestowed Aura must announce its host exactly once"
    );
}
