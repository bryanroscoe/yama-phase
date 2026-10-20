use engine::ai_support::{
    copy_effect_adds_flying, copy_target_filter, copy_target_mana_value_ceiling, find_copy_targets,
    project_copy_mana_spent_for_x,
};
use engine::game::game_object::GameObject;
use engine::types::ability::{
    AbilityDefinition, ContinuousModification, CopyRecipient, Effect, PtValue, QuantityExpr,
    TargetRef, TriggerCondition, TriggerDefinition,
};
use engine::types::actions::GameAction;
use engine::types::card_type::{CoreType, Supertype};
use engine::types::counter::{CounterMatch, CounterType};
use engine::types::game_state::{GameState, PendingCast, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::keywords::Keyword;
use engine::types::player::PlayerId;
use engine::types::triggers::TriggerMode;
use engine::types::zones::Zone;

use crate::eval::{evaluate_creature, strategic_intent, StrategicIntent};
use crate::features::DeckFeatures;

use super::activation::turn_only;
use super::context::PolicyContext;
use super::registry::{
    rescale_into_critical_band, DecisionKind, PolicyId, PolicyReason, PolicyVerdict, TacticalPolicy,
};

pub struct CopyValuePolicy;

const COPY_SPELL_LOOP_PENALTY_SCALE: f64 = 0.004;

/// Max expected magnitude of `CopyValuePolicy::score()` — the `+100` preferred-X
/// anchor plus a copy-target evaluation/penalty tail (~±30, `score_target_choice`
/// / `score_legend_rule_keep`). Sets where `rescale_into_critical_band` starts to
/// saturate; below it, ordering is preserved.
const COPY_VALUE_RAW_CEILING: f64 = 130.0;

impl CopyValuePolicy {
    pub fn score(&self, ctx: &PolicyContext<'_>) -> f64 {
        match (&ctx.decision.waiting_for, &ctx.candidate.action) {
            (
                WaitingFor::ChooseXValue {
                    pending_cast, max, ..
                },
                GameAction::ChooseX { value },
            ) => score_choose_x(ctx, pending_cast, *max, *value),
            (
                WaitingFor::CopyTargetChoice {
                    source_id,
                    valid_targets,
                    ..
                },
                GameAction::ChooseTarget {
                    target: Some(TargetRef::Object(target_id)),
                },
            ) if valid_targets.contains(target_id) => {
                score_target_choice(ctx.state, ctx.ai_player, *source_id, *target_id)
            }
            (
                WaitingFor::TargetSelection { .. } | WaitingFor::TriggerTargetSelection { .. },
                GameAction::ChooseTarget {
                    target: Some(TargetRef::Object(target_id)),
                },
            ) if ctx
                .effects()
                .iter()
                .any(|e| matches!(e, Effect::CopyTokenOf { .. })) =>
            {
                let source_id = ctx.source_object().map(|source| source.id);
                let strips = ctx
                    .effects()
                    .iter()
                    .any(|e| copy_effect_strips_legendary(e));
                score_copy_token_target(ctx.state, ctx.ai_player, source_id, *target_id, strips)
            }
            (
                WaitingFor::TargetSelection { .. } | WaitingFor::TriggerTargetSelection { .. },
                GameAction::ChooseTarget {
                    target: Some(TargetRef::Object(target_id)),
                },
            ) if ctx.effects().iter().any(|e| is_self_copy_effect(e)) => {
                let source_id = ctx.source_object().map(|source| source.id);
                let strips = ctx
                    .effects()
                    .iter()
                    .any(|e| copy_effect_strips_legendary(e));
                score_become_copy_target(ctx.state, ctx.ai_player, source_id, *target_id, strips)
            }
            _ => 0.0,
        }
    }
}

/// CR 707.2: "~ becomes a copy of target <permanent>": the ability's source is
/// the one that changes ([`CopyRecipient::Source`]); the target is only read.
pub(crate) fn is_self_copy_effect(effect: &Effect) -> bool {
    matches!(
        effect,
        Effect::BecomeCopy {
            recipient: CopyRecipient::Source,
            ..
        }
    )
}

/// CR 707.2: a permanent that "becomes a copy of target <permanent>" takes the
/// target's copiable values and leaves the target untouched, so a target is
/// worth what a copy of it is worth, whoever controls it. Creatures are valued
/// as for token copies. A noncreature's copy is valued by the payoff of a state
/// trigger the copy satisfies at once ([`zero_counter_state_trigger_payoff`]);
/// without one it scores 0, as before.
fn score_become_copy_target(
    state: &GameState,
    ai_player: PlayerId,
    source_id: Option<ObjectId>,
    target_id: ObjectId,
    strips_legendary: bool,
) -> f64 {
    let Some(target) = state.objects.get(&target_id) else {
        return -10.0;
    };
    let payoff = source_id
        .and_then(|id| state.objects.get(&id))
        .map_or(0.0, |source| {
            zero_counter_state_trigger_payoff(source, target)
        });
    if payoff > 0.0 {
        // CR 704.5j: copying one's own legend still forces the legend rule, but
        // here the copy is the one to keep (it carries the payoff), so the cost
        // is the original the AI puts into its graveyard, not a wasted copy.
        let lost_original = if target.controller == ai_player
            && target.card_types.supertypes.contains(&Supertype::Legendary)
            && !strips_legendary
        {
            evaluate_legend_keep_permanent(state, target_id, target)
        } else {
            0.0
        };
        return payoff
            - lost_original
            - copy_target_penalties(state, ai_player, source_id, target, true);
    }
    score_copy_token_target(state, ai_player, source_id, target_id, strips_legendary)
}

/// CR 603.8 + CR 707.2 + CR 614.12: the value of the creature tokens made by
/// `target`'s "when ~ has no <kind> counters on it" state triggers, for a
/// `holder` of its copiable values that carries none of those counters.
///
/// Counters aren't copiable values, and a permanent that becomes a copy
/// without entering the battlefield never gets the target's "enters with N
/// counters" replacement, so such a trigger fires as soon as the copy exists:
/// Thespian's Stage becoming a copy of Dark Depths has no ice counters, so it
/// is sacrificed for Marit Lage, a 20/20 flying indestructible token. With
/// `holder == target` this reads whether a permanent's own payoff is live
/// (the copy to keep at the legend rule). Returns 0 when nothing applies.
pub(crate) fn zero_counter_state_trigger_payoff(holder: &GameObject, target: &GameObject) -> f64 {
    let payoff: f64 = target
        .trigger_definitions
        .iter_unchecked()
        .map(|entry| &entry.definition)
        .filter(|trigger| {
            state_trigger_counter_kind(trigger)
                .is_some_and(|kind| holder.counters.get(kind).copied().unwrap_or(0) == 0)
        })
        .filter_map(|trigger| trigger.execute.as_deref())
        .map(created_creature_token_value)
        .sum();
    payoff.min(30.0)
}

/// The counter kind of a "When ~ has no <kind> counters on it" state trigger
/// (CR 603.8), whose condition is "at most zero counters of that kind".
fn state_trigger_counter_kind(trigger: &TriggerDefinition) -> Option<&CounterType> {
    if trigger.mode != TriggerMode::StateCondition {
        return None;
    }
    let mut condition = trigger.condition.as_ref()?;
    while let TriggerCondition::EventTime { condition: inner } = condition {
        condition = inner;
    }
    match condition {
        TriggerCondition::HasCounters {
            counters: CounterMatch::OfType(kind),
            maximum: Some(0),
            ..
        } => Some(kind),
        _ => None,
    }
}

/// A rough value for the creature tokens an ability creates: half the fixed
/// power plus toughness, and a little for each evasion or protection keyword,
/// per token. Tokens with dynamic stats count 1 each.
fn created_creature_token_value(ability: &AbilityDefinition) -> f64 {
    crate::cast_facts::collect_definition_effects(ability)
        .into_iter()
        .map(|effect| match effect {
            Effect::Token {
                power,
                toughness,
                types,
                keywords,
                count,
                ..
            } if types.iter().any(|t| t.eq_ignore_ascii_case("creature")) => {
                let stats = match (power, toughness) {
                    (PtValue::Fixed(p), PtValue::Fixed(t)) => f64::from((p + t).max(0)) / 2.0,
                    _ => 1.0,
                };
                let keyword_bonus = keywords
                    .iter()
                    .filter(|k| {
                        matches!(
                            k,
                            Keyword::Flying
                                | Keyword::Trample
                                | Keyword::Indestructible
                                | Keyword::Hexproof
                                | Keyword::Menace
                                | Keyword::Haste
                        )
                    })
                    .count() as f64
                    * 1.5;
                let n = match count {
                    QuantityExpr::Fixed { value } => f64::from((*value).max(0)),
                    _ => 1.0,
                };
                n * (stats + keyword_bonus)
            }
            _ => 0.0,
        })
        .sum()
}

/// CR 704.5j: Prefer keeping commanders and non-token originals over ephemeral
/// copy tokens when the legend rule fires.
pub(crate) fn score_legend_rule_keep(state: &GameState, keep: ObjectId) -> f64 {
    let Some(object) = state.objects.get(&keep) else {
        return -100.0;
    };
    let mut score = evaluate_legend_keep_permanent(state, keep, object);
    if object.is_commander {
        score += 80.0;
    }
    if object.is_token {
        score -= 60.0;
    }
    score
}

fn evaluate_legend_keep_permanent(state: &GameState, keep: ObjectId, object: &GameObject) -> f64 {
    if object.card_types.core_types.contains(&CoreType::Creature) {
        return evaluate_creature(state, keep);
    }

    if object
        .card_types
        .core_types
        .contains(&CoreType::Planeswalker)
    {
        return object.mana_cost.mana_value() as f64 + 2.0;
    }

    if object.card_types.core_types.contains(&CoreType::Land) {
        // Keep the copy whose "no counters" state trigger is live (Thespian's
        // Stage as a copy of Dark Depths) over the original that still has
        // its counters; otherwise lands tie.
        return 3.0 + zero_counter_state_trigger_payoff(object, object);
    }

    (object.mana_cost.mana_value() as f64).min(6.0)
}

/// CR 707.9 + CR 205.4a: A copy inherits the copied object's supertypes
/// (including Legendary) unless the copy effect includes an "except" clause that
/// strips it (Miirym, Sentinel Wyrm / Helm of the Host: "except the token isn't
/// legendary" → `RemoveSupertype { Legendary }`). Core-type modifications
/// (`RemoveType`, `SetCardTypes`; CR 205.1a) act on card *types*, not
/// supertypes, so they can never remove Legendary — a `RemoveSupertype {
/// Legendary }` in `additional_modifications` is the sole legend-strip. Handles
/// both the token-copy (`CopyTokenOf`) and enters-as-copy (`BecomeCopy`) forms,
/// which share the field.
pub(crate) fn copy_effect_strips_legendary(effect: &Effect) -> bool {
    let mods = match effect {
        Effect::CopyTokenOf {
            additional_modifications,
            ..
        }
        | Effect::BecomeCopy {
            additional_modifications,
            ..
        } => additional_modifications,
        _ => return false,
    };
    mods.iter().any(|m| {
        matches!(
            m,
            ContinuousModification::RemoveSupertype {
                supertype: Supertype::Legendary
            }
        )
    })
}

/// Penalties for copy effects that would trigger a wasteful legend-rule loop
/// (issue #2438 — Saheeli copying her own commander). `strips_legendary` is the
/// copy effect's typed decision (from `copy_effect_strips_legendary`): a copy
/// that makes its token non-legendary creates no legend-rule collision.
pub(crate) fn copy_target_penalties(
    state: &GameState,
    ai_player: PlayerId,
    source_id: Option<ObjectId>,
    target: &GameObject,
    strips_legendary: bool,
) -> f64 {
    let mut penalty = 0.0;

    if source_id.is_some_and(|source_id| target.id == source_id) {
        penalty += 50.0;
    }

    if target.is_commander && target.controller == ai_player {
        penalty += 40.0;
    }

    // CR 704.5j: copying your own legendary CREATES a same-name legend-rule
    // collision (the copy is the duplicate that forces one into the graveyard).
    // Penalize unless the copy makes the token non-legendary
    // (`RemoveSupertype { Legendary }`) or the legend rule is switched off for
    // the target (Mirror Gallery / Sakashima, `legend_rule_exempt`).
    if target.controller == ai_player
        && target.card_types.supertypes.contains(&Supertype::Legendary)
        && !strips_legendary
        && !engine::game::sba::legend_rule_exempt(state, target.id)
    {
        penalty += 35.0;
    }

    penalty
}

fn score_copy_token_target(
    state: &GameState,
    ai_player: PlayerId,
    source_id: Option<ObjectId>,
    target_id: ObjectId,
    strips_legendary: bool,
) -> f64 {
    let Some(target) = state.objects.get(&target_id) else {
        return -10.0;
    };
    let base = evaluate_creature(state, target_id);
    let penalty = copy_target_penalties(state, ai_player, source_id, target, strips_legendary);
    base - penalty
}

impl TacticalPolicy for CopyValuePolicy {
    fn id(&self) -> PolicyId {
        PolicyId::CopyValue
    }

    fn decision_kinds(&self) -> &'static [DecisionKind] {
        &[DecisionKind::ChooseX, DecisionKind::SelectTarget]
    }

    fn activation(
        &self,
        features: &DeckFeatures,
        state: &GameState,
        _player: PlayerId,
    ) -> Option<f32> {
        turn_only(features, state)
    }

    fn verdict(&self, ctx: &PolicyContext<'_>) -> PolicyVerdict {
        // `score()` is a wide analog signal (the +100 "preferred X" anchor plus
        // copy-target penalty sums reach ~±125) also consumed raw by other
        // policies. As a *verdict* it must obey the score contract — but routing
        // ±125 straight through PolicyVerdict::score would SATURATE, collapsing
        // every large candidate to the same ±CRITICAL_MAX and flattening the
        // copy/ChooseX/legend move-ordering prior. Rescale the range into the
        // band first so ordering survives, then band-dispatch (issue #5473).
        PolicyVerdict::score(
            rescale_into_critical_band(self.score(ctx), COPY_VALUE_RAW_CEILING),
            PolicyReason::new("copy_value_score"),
        )
    }
}

fn score_choose_x(
    ctx: &PolicyContext<'_>,
    pending_cast: &PendingCast,
    max_x: u32,
    candidate_x: u32,
) -> f64 {
    let Some(source) = ctx.state.objects.get(&pending_cast.object_id) else {
        return 0.0;
    };
    let Some(effect_def) = copy_effect_for_object(source) else {
        return 0.0;
    };

    let scores: Vec<_> = (0..=max_x)
        .map(|x_value| {
            let projected_spent = project_copy_mana_spent_for_x(pending_cast, x_value);
            let ceiling = copy_target_mana_value_ceiling(projected_spent, effect_def);
            let best_target =
                legal_copy_targets(ctx.state, source.id, source.controller, effect_def, ceiling)
                    .into_iter()
                    .map(|target_id| {
                        score_target_choice(ctx.state, ctx.ai_player, source.id, target_id)
                    })
                    .max_by(|left, right| left.total_cmp(right))
                    .unwrap_or(0.10);
            let raw = best_target - (0.03 * x_value as f64);
            (x_value, raw)
        })
        .collect();

    let preferred_x = preferred_x_value(&scores);
    let raw_score = scores
        .iter()
        .find(|(x_value, _)| *x_value == candidate_x)
        .map(|(_, score)| *score)
        .unwrap_or(0.0);

    if candidate_x == preferred_x {
        100.0 + raw_score
    } else {
        raw_score
    }
}

fn preferred_x_value(scores: &[(u32, f64)]) -> u32 {
    let mut best = None;

    for &(x_value, score) in scores {
        best = match best {
            None => Some((x_value, score)),
            Some((best_x, best_score)) => {
                if score > best_score + 0.05
                    || ((score - best_score).abs() <= 0.05 && x_value < best_x)
                {
                    Some((x_value, score))
                } else {
                    Some((best_x, best_score))
                }
            }
        };
    }

    best.map(|(x_value, _)| x_value).unwrap_or(0)
}

fn score_target_choice(
    state: &GameState,
    ai_player: PlayerId,
    source_id: ObjectId,
    target_id: ObjectId,
) -> f64 {
    let Some(source) = state.objects.get(&source_id) else {
        return 0.0;
    };
    let Some(target) = state.objects.get(&target_id) else {
        return 0.0;
    };
    let Some(effect_def) = copy_effect_for_object(source) else {
        return 0.0;
    };

    let base_creature_value = evaluate_creature(state, target_id);
    let mut copy_bonus = 0.0;
    let mut copy_penalty = 0.0;

    if target_has_etb_value(target) {
        copy_bonus += 0.12;
    }

    if copy_effect_adds_flying(effect_def)
        && !target.has_keyword(&Keyword::Flying)
        && strategic_intent(state, ai_player) != StrategicIntent::Stabilize
        && target.power.unwrap_or(0) > 0
    {
        copy_bonus += 0.08;
    }

    if target.controller == ai_player && strengthens_supported_plan(state, ai_player, target) {
        copy_bonus += 0.06;
    }

    let strips = copy_effect_strips_legendary(&effect_def.effect);
    copy_penalty += copy_target_penalties(state, ai_player, Some(source_id), target, strips)
        * COPY_SPELL_LOOP_PENALTY_SCALE;

    if base_creature_value < 3.0 {
        copy_penalty += 0.08;
    }

    base_creature_value + copy_bonus - copy_penalty
}

fn copy_effect_for_object(
    object: &engine::game::game_object::GameObject,
) -> Option<&AbilityDefinition> {
    object
        .replacement_definitions
        .iter_unchecked()
        .filter_map(|replacement| replacement.execute.as_deref())
        .find(|effect_def| copy_target_filter(effect_def).is_some())
}

fn legal_copy_targets(
    state: &GameState,
    source_id: ObjectId,
    controller: PlayerId,
    effect_def: &AbilityDefinition,
    max_mana_value: Option<u32>,
) -> Vec<ObjectId> {
    let Some(filter) = copy_target_filter(effect_def) else {
        return Vec::new();
    };

    // Delegates to the engine's copy-source authority, which owns which zone
    // the copy filter names (graveyard for Body Double, exile for The
    // Mimeoplasm, battlefield by default) and measures the mana-value ceiling
    // with `effective_mana_value` (a split card counts both halves).
    find_copy_targets(state, filter, source_id, controller, max_mana_value)
}

fn target_has_etb_value(object: &engine::game::game_object::GameObject) -> bool {
    object
        .trigger_definitions
        .iter_unchecked()
        .map(|entry| &entry.definition)
        .any(|trigger| {
            trigger.mode == TriggerMode::ChangesZone
                && trigger.destination == Some(Zone::Battlefield)
        })
}

fn strengthens_supported_plan(
    state: &GameState,
    ai_player: PlayerId,
    object: &engine::game::game_object::GameObject,
) -> bool {
    match strategic_intent(state, ai_player) {
        StrategicIntent::PushLethal
        | StrategicIntent::PreserveAdvantage
        | StrategicIntent::Develop => {
            object.power.unwrap_or(0) >= 3
                || object.has_keyword(&Keyword::Flying)
                || object.has_keyword(&Keyword::Trample)
                || object.has_keyword(&Keyword::Menace)
                || !object.abilities.is_empty()
                || !object.trigger_definitions.is_empty()
        }
        StrategicIntent::Stabilize => {
            object.toughness.unwrap_or(0) >= 4
                || object.has_keyword(&Keyword::Deathtouch)
                || object.has_keyword(&Keyword::Lifelink)
                || object.has_keyword(&Keyword::Vigilance)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::ai_support::{ActionMetadata, AiDecisionContext, CandidateAction, TacticalClass};
    use engine::game::zones::create_object;
    use engine::types::ability::{
        AbilityKind, ContinuousModification, CopyManaValueLimit, Effect, QuantityExpr,
        ReplacementDefinition, StaticDefinition, TargetFilter,
    };
    use engine::types::card_type::CoreType;
    use engine::types::game_state::PendingCast;
    use engine::types::identifiers::CardId;
    use engine::types::mana::{ManaCost, ManaCostShard};
    use engine::types::replacements::ReplacementEvent;
    use engine::types::statics::StaticMode;

    fn make_state() -> GameState {
        let mut state = GameState::new_two_player(42);
        state.active_player = PlayerId(0);
        state.priority_player = PlayerId(0);
        state
    }

    fn add_mockingbird_like_card(state: &mut GameState, zone: Zone) -> ObjectId {
        let object_id = create_object(
            state,
            CardId(100),
            PlayerId(0),
            "Mockingbird".to_string(),
            zone,
        );
        let object = state.objects.get_mut(&object_id).unwrap();
        object.card_types.core_types.push(CoreType::Creature);
        object.power = Some(1);
        object.toughness = Some(1);
        object.base_power = Some(1);
        object.base_toughness = Some(1);
        object.base_keywords.push(Keyword::Flying);
        object.keywords.push(Keyword::Flying);
        object.replacement_definitions.push(
            ReplacementDefinition::new(ReplacementEvent::Moved).execute(AbilityDefinition::new(
                AbilityKind::Spell,
                Effect::BecomeCopy {
                    target: TargetFilter::Any,
                    recipient: engine::types::ability::CopyRecipient::Source,
                    duration: None,
                    mana_value_limit: Some(CopyManaValueLimit::AmountSpentToCastSource),
                    additional_modifications: vec![
                        ContinuousModification::AddSubtype {
                            subtype: "Bird".to_string(),
                        },
                        ContinuousModification::AddKeyword {
                            keyword: Keyword::Flying,
                        },
                    ],
                },
            )),
        );
        object_id
    }

    fn add_creature(
        state: &mut GameState,
        card_id: u64,
        owner: PlayerId,
        name: &str,
        power: i32,
        toughness: i32,
        mana_value: u32,
    ) -> ObjectId {
        let object_id = create_object(
            state,
            CardId(card_id),
            owner,
            name.to_string(),
            Zone::Battlefield,
        );
        let object = state.objects.get_mut(&object_id).unwrap();
        object.card_types.core_types.push(CoreType::Creature);
        object.power = Some(power);
        object.toughness = Some(toughness);
        object.base_power = Some(power);
        object.base_toughness = Some(toughness);
        object.mana_cost = ManaCost::generic(mana_value);
        object.card_types.supertypes.retain(|_| false);
        object_id
    }

    #[test]
    fn choose_x_prefers_smallest_value_when_no_copy_targets_exist() {
        let mut state = make_state();
        let mockingbird_id = add_mockingbird_like_card(&mut state, Zone::Hand);
        let pending_cast = PendingCast::new(
            mockingbird_id,
            CardId(100),
            engine::types::ability::ResolvedAbility::new(
                Effect::Draw {
                    count: QuantityExpr::Fixed { value: 1 },
                    target: engine::types::ability::TargetFilter::Controller,
                },
                Vec::new(),
                mockingbird_id,
                PlayerId(0),
            ),
            ManaCost::Cost {
                shards: vec![ManaCostShard::X, ManaCostShard::Blue],
                generic: 0,
            },
        );
        let decision = AiDecisionContext {
            waiting_for: WaitingFor::ChooseXValue {
                player: PlayerId(0),
                min: 0,
                max: 3,
                pending_cast: Box::new(pending_cast),
                convoke_mode: None,
                x_cost_previews: vec![],
            },
            candidates: Vec::new(),
        };

        let score_zero = CopyValuePolicy.score(&PolicyContext {
            state: &state,
            decision: &decision,
            candidate: &CandidateAction {
                action: GameAction::ChooseX { value: 0 },
                metadata: ActionMetadata::for_actor(Some(PlayerId(0)), TacticalClass::Selection),
            },
            ai_player: PlayerId(0),
            config: &crate::config::AiConfig::default(),
            context: &crate::context::AiContext::empty(&crate::eval::EvalWeightSet::default()),
            cast_facts: None,
            search_depth: crate::policies::context::SearchDepth::Root,
        });
        let score_two = CopyValuePolicy.score(&PolicyContext {
            state: &state,
            decision: &decision,
            candidate: &CandidateAction {
                action: GameAction::ChooseX { value: 2 },
                metadata: ActionMetadata::for_actor(Some(PlayerId(0)), TacticalClass::Selection),
            },
            ai_player: PlayerId(0),
            config: &crate::config::AiConfig::default(),
            context: &crate::context::AiContext::empty(&crate::eval::EvalWeightSet::default()),
            cast_facts: None,
            search_depth: crate::policies::context::SearchDepth::Root,
        });

        assert!(score_zero > score_two);
    }

    #[test]
    fn choose_x_unlocks_higher_mana_value_target_when_materially_better() {
        let mut state = make_state();
        let mockingbird_id = add_mockingbird_like_card(&mut state, Zone::Hand);
        add_creature(&mut state, 1, PlayerId(0), "Otter", 1, 1, 1);
        add_creature(&mut state, 2, PlayerId(1), "Dragon", 4, 4, 4);
        let pending_cast = PendingCast::new(
            mockingbird_id,
            CardId(100),
            engine::types::ability::ResolvedAbility::new(
                Effect::Draw {
                    count: QuantityExpr::Fixed { value: 1 },
                    target: engine::types::ability::TargetFilter::Controller,
                },
                Vec::new(),
                mockingbird_id,
                PlayerId(0),
            ),
            ManaCost::Cost {
                shards: vec![ManaCostShard::X, ManaCostShard::Blue],
                generic: 0,
            },
        );
        let decision = AiDecisionContext {
            waiting_for: WaitingFor::ChooseXValue {
                player: PlayerId(0),
                min: 0,
                max: 3,
                pending_cast: Box::new(pending_cast),
                convoke_mode: None,
                x_cost_previews: vec![],
            },
            candidates: Vec::new(),
        };

        let score_zero = CopyValuePolicy.score(&PolicyContext {
            state: &state,
            decision: &decision,
            candidate: &CandidateAction {
                action: GameAction::ChooseX { value: 0 },
                metadata: ActionMetadata::for_actor(Some(PlayerId(0)), TacticalClass::Selection),
            },
            ai_player: PlayerId(0),
            config: &crate::config::AiConfig::default(),
            context: &crate::context::AiContext::empty(&crate::eval::EvalWeightSet::default()),
            cast_facts: None,
            search_depth: crate::policies::context::SearchDepth::Root,
        });
        let score_three = CopyValuePolicy.score(&PolicyContext {
            state: &state,
            decision: &decision,
            candidate: &CandidateAction {
                action: GameAction::ChooseX { value: 3 },
                metadata: ActionMetadata::for_actor(Some(PlayerId(0)), TacticalClass::Selection),
            },
            ai_player: PlayerId(0),
            config: &crate::config::AiConfig::default(),
            context: &crate::context::AiContext::empty(&crate::eval::EvalWeightSet::default()),
            cast_facts: None,
            search_depth: crate::policies::context::SearchDepth::Root,
        });

        assert!(score_three > score_zero);
    }

    #[test]
    fn copy_target_choice_prefers_higher_value_target() {
        let mut state = make_state();
        let mockingbird_id = add_mockingbird_like_card(&mut state, Zone::Battlefield);
        let small = add_creature(&mut state, 1, PlayerId(1), "Mouse", 1, 1, 1);
        let large = add_creature(&mut state, 2, PlayerId(1), "Dragon", 4, 4, 4);
        let decision = AiDecisionContext {
            waiting_for: WaitingFor::CopyTargetChoice {
                player: PlayerId(0),
                source_id: mockingbird_id,
                valid_targets: vec![small, large],
                max_mana_value: Some(4),
                purpose: engine::types::ability::CopyTargetPurpose::BecomeCopy,
            },
            candidates: Vec::new(),
        };

        let score_small = CopyValuePolicy.score(&PolicyContext {
            state: &state,
            decision: &decision,
            candidate: &CandidateAction {
                action: GameAction::ChooseTarget {
                    target: Some(TargetRef::Object(small)),
                },
                metadata: ActionMetadata::for_actor(Some(PlayerId(0)), TacticalClass::Selection),
            },
            ai_player: PlayerId(0),
            config: &crate::config::AiConfig::default(),
            context: &crate::context::AiContext::empty(&crate::eval::EvalWeightSet::default()),
            cast_facts: None,
            search_depth: crate::policies::context::SearchDepth::Root,
        });
        let score_large = CopyValuePolicy.score(&PolicyContext {
            state: &state,
            decision: &decision,
            candidate: &CandidateAction {
                action: GameAction::ChooseTarget {
                    target: Some(TargetRef::Object(large)),
                },
                metadata: ActionMetadata::for_actor(Some(PlayerId(0)), TacticalClass::Selection),
            },
            ai_player: PlayerId(0),
            config: &crate::config::AiConfig::default(),
            context: &crate::context::AiContext::empty(&crate::eval::EvalWeightSet::default()),
            cast_facts: None,
            search_depth: crate::policies::context::SearchDepth::Root,
        });

        assert!(score_large > score_small);
    }

    #[test]
    fn copy_spell_target_choice_scales_loop_penalty_to_fractional_score() {
        let mut state = make_state();
        let mockingbird_id = add_mockingbird_like_card(&mut state, Zone::Battlefield);
        {
            let obj = state.objects.get_mut(&mockingbird_id).unwrap();
            obj.card_types.supertypes.push(Supertype::Legendary);
            obj.is_commander = true;
        }

        let score = score_target_choice(&state, PlayerId(0), mockingbird_id, mockingbird_id);
        assert!(
            score > 0.0,
            "copy-spell loop penalty must stay on the fractional target-score scale, got {score}"
        );
    }

    #[test]
    fn copy_token_target_heavily_penalises_self_commander() {
        let mut state = make_state();
        let saheeli = add_creature(
            &mut state,
            1,
            PlayerId(0),
            "Saheeli, Radiant Creator",
            2,
            3,
            4,
        );
        {
            let obj = state.objects.get_mut(&saheeli).unwrap();
            obj.card_types.supertypes.push(Supertype::Legendary);
            obj.is_commander = true;
        }

        let score = score_copy_token_target(&state, PlayerId(0), Some(saheeli), saheeli, false);
        assert!(
            score < -50.0,
            "self-commander copy must be strongly penalised, got {score}"
        );
    }

    #[test]
    fn copy_token_target_without_source_does_not_apply_self_copy_penalty() {
        let mut state = make_state();
        let saheeli = add_creature(
            &mut state,
            1,
            PlayerId(0),
            "Saheeli, Radiant Creator",
            2,
            3,
            4,
        );
        {
            let obj = state.objects.get_mut(&saheeli).unwrap();
            obj.card_types.supertypes.push(Supertype::Legendary);
            obj.is_commander = true;
        }

        let unknown_source_score =
            score_copy_token_target(&state, PlayerId(0), None, saheeli, false);
        let self_source_score =
            score_copy_token_target(&state, PlayerId(0), Some(saheeli), saheeli, false);
        assert!(
            unknown_source_score > self_source_score + 45.0,
            "unknown source must not be treated as self-copy: unknown={unknown_source_score}, self={self_source_score}"
        );
    }

    fn legendary_creature(state: &mut GameState, card_id: u64, name: &str) -> ObjectId {
        let id = add_creature(state, card_id, PlayerId(0), name, 3, 3, 4);
        state
            .objects
            .get_mut(&id)
            .unwrap()
            .card_types
            .supertypes
            .push(Supertype::Legendary);
        id
    }

    /// Mirror Gallery: a global `LegendRuleDoesntApply` static that exempts every
    /// legendary permanent from the legend rule (CR 704.5j).
    fn add_mirror_gallery(state: &mut GameState) {
        let id = create_object(
            state,
            CardId(900),
            PlayerId(0),
            "Mirror Gallery".to_string(),
            Zone::Battlefield,
        );
        let mut def = StaticDefinition::new(StaticMode::LegendRuleDoesntApply);
        def.affected = None;
        state
            .objects
            .get_mut(&id)
            .unwrap()
            .static_definitions
            .push(def);
    }

    #[test]
    fn copy_own_lone_legendary_is_penalised() {
        // Step 4: copying your own legendary CREATES a same-name legend-rule
        // collision (the copy is the duplicate), so a lone own legendary — with NO
        // pre-existing same-name duplicate on the battlefield — must be penalised.
        // The old same-name-duplicate branch returned 0 here; reverting to it
        // flips this assertion.
        let mut state = make_state();
        let target_id = legendary_creature(&mut state, 1, "Adeline, Resplendent Cathar");
        let target = &state.objects[&target_id];
        let penalty = copy_target_penalties(&state, PlayerId(0), None, target, false);
        assert!(
            penalty >= 35.0,
            "own legendary copy must be penalised even with no pre-existing duplicate, got {penalty}"
        );
    }

    #[test]
    fn legendary_strip_copy_not_penalised() {
        // Hostile fixture (i): a legend-stripping copy (Miirym: "except the token
        // isn't legendary" → strips_legendary=true) creates a non-legendary token,
        // so no collision → no legendary penalty.
        let mut state = make_state();
        let target_id = legendary_creature(&mut state, 1, "Adeline, Resplendent Cathar");
        let target = &state.objects[&target_id];
        let penalty = copy_target_penalties(&state, PlayerId(0), None, target, true);
        assert_eq!(
            penalty, 0.0,
            "legend-stripping copy of own legendary must not be penalised, got {penalty}"
        );
    }

    #[test]
    fn legend_exempt_copy_not_penalised() {
        // Hostile fixture (iii): Mirror Gallery switches off the legend rule, so a
        // duplicate legendary is legal → no penalty (`legend_rule_exempt`).
        let mut state = make_state();
        add_mirror_gallery(&mut state);
        let target_id = legendary_creature(&mut state, 1, "Adeline, Resplendent Cathar");
        let target = &state.objects[&target_id];
        let penalty = copy_target_penalties(&state, PlayerId(0), None, target, false);
        assert_eq!(
            penalty, 0.0,
            "legend-rule-exempt legendary (Mirror Gallery) must not be penalised, got {penalty}"
        );
    }

    #[test]
    fn own_nonlegendary_copy_not_penalised() {
        // Reach-guard: a non-legendary own target has no legend-rule concern → 0.
        let mut state = make_state();
        let target_id = add_creature(&mut state, 1, PlayerId(0), "Grizzly Bears", 2, 2, 2);
        let target = &state.objects[&target_id];
        let penalty = copy_target_penalties(&state, PlayerId(0), None, target, false);
        assert_eq!(
            penalty, 0.0,
            "own non-legendary copy target must not be penalised, got {penalty}"
        );
    }

    #[test]
    fn own_nonlegendary_copy_target_outscores_own_legendary() {
        // Hostile fixture (ii): given an own legendary and an own non-legendary of
        // equal stats, the non-legendary target must outscore the legendary one
        // (which eats the +35 legend-rule-collision penalty). A non-legendary copy
        // merely outscores — the legendary is never hard-rejected.
        let mut state = make_state();
        let legendary = legendary_creature(&mut state, 1, "Adeline, Resplendent Cathar");
        let vanilla = add_creature(&mut state, 2, PlayerId(0), "Adeline's Understudy", 3, 3, 4);

        let legendary_score = score_copy_token_target(&state, PlayerId(0), None, legendary, false);
        let vanilla_score = score_copy_token_target(&state, PlayerId(0), None, vanilla, false);
        assert!(
            vanilla_score > legendary_score,
            "own non-legendary copy target must outscore own legendary: vanilla={vanilla_score}, legendary={legendary_score}"
        );
    }

    #[test]
    fn legend_rule_keep_scores_noncreature_permanent_value() {
        let mut state = make_state();
        let artifact = create_object(
            &mut state,
            CardId(1),
            PlayerId(0),
            "The One Ring".to_string(),
            Zone::Battlefield,
        );
        {
            let obj = state.objects.get_mut(&artifact).unwrap();
            obj.card_types.core_types.push(CoreType::Artifact);
            obj.card_types.supertypes.push(Supertype::Legendary);
            obj.mana_cost = ManaCost::generic(4);
        }

        let score = score_legend_rule_keep(&state, artifact);
        assert!(
            score >= 4.0,
            "legend-rule keep score should value noncreature permanents, got {score}"
        );
    }

    #[test]
    fn legend_rule_keep_prefers_commander_over_copy_token() {
        let mut state = make_state();
        let commander = add_creature(
            &mut state,
            1,
            PlayerId(0),
            "Saheeli, Radiant Creator",
            2,
            3,
            4,
        );
        {
            let obj = state.objects.get_mut(&commander).unwrap();
            obj.card_types.supertypes.push(Supertype::Legendary);
            obj.is_commander = true;
        }
        let copy_token = add_creature(
            &mut state,
            2,
            PlayerId(0),
            "Saheeli, Radiant Creator",
            5,
            5,
            4,
        );
        {
            let obj = state.objects.get_mut(&copy_token).unwrap();
            obj.card_types.supertypes.push(Supertype::Legendary);
            obj.is_token = true;
        }

        let commander_score = score_legend_rule_keep(&state, commander);
        let token_score = score_legend_rule_keep(&state, copy_token);
        assert!(
            commander_score > token_score,
            "commander ({commander_score}) must beat copy token ({token_score})"
        );
    }
}
