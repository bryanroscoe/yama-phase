//! `SweeperTimingPolicy` — bias toward casting board wipes when they are
//! impactful (≥3 opposing creatures) and against casting them prematurely.
//!
//! CR 608.2: sweepers resolve and apply their effect to all matching permanents
//! at the time of resolution. CR 701.8: destroy moves permanents to the
//! graveyard. CR 701.13: exile removes them from the game entirely.

use std::collections::HashMap;

use engine::game::filter::{matches_target_filter, FilterContext};
use engine::game::game_object::GameObject;
use engine::game::players;
use engine::types::card_type::CoreType;
use engine::types::game_state::GameState;
use engine::types::player::PlayerId;

use super::context::PolicyContext;
use super::registry::{DecisionKind, PolicyId, PolicyReason, PolicyVerdict, TacticalPolicy};
use crate::cast_facts::is_cast_family_action;
use crate::features::control::{sweeper_filters, COMMITMENT_FLOOR};
use crate::features::DeckFeatures;
#[cfg(test)]
use engine::types::actions::GameAction;
#[cfg(test)]
use engine::types::game_state::CastPaymentMode;

/// Opponent creature threshold — at this count a sweeper is "timely".
const SWEEPER_TIMELY_THRESHOLD: u32 = 3;
/// Score bonus when the sweeper clears a meaningful board.
const DELTA_TIMELY: f64 = 1.5;
/// Score penalty when the sweeper would hit fewer than the threshold.
const DELTA_PREMATURE: f64 = -2.0;

pub struct SweeperTimingPolicy;

impl TacticalPolicy for SweeperTimingPolicy {
    fn id(&self) -> PolicyId {
        PolicyId::SweeperTiming
    }

    fn decision_kinds(&self) -> &'static [DecisionKind] {
        &[DecisionKind::CastSpell]
    }

    fn activation(
        &self,
        features: &DeckFeatures,
        _state: &GameState,
        _player: PlayerId,
    ) -> Option<f32> {
        if features.control.sweeper_count == 0 {
            return None;
        }
        if features.control.commitment < COMMITMENT_FLOOR {
            return None;
        }
        Some(features.control.commitment)
    }

    fn verdict(&self, ctx: &PolicyContext<'_>) -> PolicyVerdict {
        // Only applies to sweeper spells. Non-sweepers get a neutral verdict.
        let Some(sweeper) = candidate_sweeper(ctx) else {
            return PolicyVerdict::Score {
                delta: 0.0,
                reason: PolicyReason::new("sweeper_timing_na"),
            };
        };

        let creatures = creatures_by_controller(ctx.state);
        let creatures_of = |player: &PlayerId| creatures.get(player).copied().unwrap_or(0);
        let opponents = players::opponents(ctx.state, ctx.ai_player);
        let opposing_creatures: u32 = opponents.iter().map(creatures_of).sum();

        // Free-for-all: the threshold counts one opponent's board. Summed over
        // several opponents it is reached when each has a creature or two, and a
        // wipe then resets every board, the caster's included. Apply it to the
        // most developed opponent instead, and call a wipe that costs the caster
        // as many creatures as it costs that opponent premature.
        if opponents.len() >= 2 {
            let leader = opponents.iter().map(creatures_of).max().unwrap_or(0);
            let own_swept = own_creatures_swept(ctx.state, ctx.ai_player, sweeper);
            let timely = leader >= SWEEPER_TIMELY_THRESHOLD && own_swept < leader;
            let (delta, kind) = if timely {
                (DELTA_TIMELY, "sweeper_timely")
            } else {
                (DELTA_PREMATURE, "sweeper_premature")
            };
            return PolicyVerdict::score(
                delta,
                PolicyReason::new(kind)
                    .with_fact("opposing_creatures", opposing_creatures as i64)
                    .with_fact("leading_opponent_creatures", leader as i64)
                    .with_fact("own_creatures_swept", own_swept as i64),
            );
        }

        if opposing_creatures >= SWEEPER_TIMELY_THRESHOLD {
            PolicyVerdict::Score {
                delta: DELTA_TIMELY,
                reason: PolicyReason::new("sweeper_timely")
                    .with_fact("opposing_creatures", opposing_creatures as i64),
            }
        } else {
            PolicyVerdict::Score {
                delta: DELTA_PREMATURE,
                reason: PolicyReason::new("sweeper_premature")
                    .with_fact("opposing_creatures", opposing_creatures as i64),
            }
        }
    }
}

/// The spell the candidate casts, when it has sweeper-shaped effects.
/// Delegates to the feature module's `sweeper_filters` — single structural
/// source of truth shared between detection and runtime policy scoring.
///
/// CR 118.9: the whole cast family counts, not just the plain announcement — a
/// Terminus cast off its CR 702.94a miracle cost wipes exactly the same board.
/// The object comes from `CastFacts`, which resolves each variant's own object
/// field through the engine's action→object map.
fn candidate_sweeper<'a>(ctx: &PolicyContext<'a>) -> Option<&'a GameObject> {
    if !is_cast_family_action(&ctx.candidate.action) {
        return None;
    }
    let object = ctx.cast_facts()?.object;
    (!sweeper_filters(&object.abilities).is_empty()).then_some(object)
}

/// Creature permanents on the battlefield, counted per controller.
/// CR 608.2h: resolution checks the board state at the time of effect application.
fn creatures_by_controller(state: &GameState) -> HashMap<PlayerId, u32> {
    let mut counts = HashMap::new();
    for obj in state
        .battlefield
        .iter()
        .filter_map(|id| state.objects.get(id))
    {
        if obj.card_types.core_types.contains(&CoreType::Creature) {
            *counts.entry(obj.controller).or_insert(0) += 1;
        }
    }
    counts
}

/// The creatures `player` controls that `sweeper`'s mass effects would reach.
/// A one-sided wipe ("destroy all creatures your opponents control") reaches
/// none of the caster's own.
fn own_creatures_swept(state: &GameState, player: PlayerId, sweeper: &GameObject) -> u32 {
    let filters = sweeper_filters(&sweeper.abilities);
    let filter_ctx = FilterContext::from_source(state, sweeper.id);
    state
        .battlefield
        .iter()
        .filter(|&&id| {
            state.objects.get(&id).is_some_and(|obj| {
                obj.controller == player && obj.card_types.core_types.contains(&CoreType::Creature)
            }) && filters
                .iter()
                .any(|filter| matches_target_filter(state, id, filter, &filter_ctx))
        })
        .count() as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AiConfig;
    use crate::context::AiContext;
    use crate::features::control::ControlFeature;
    use crate::features::DeckFeatures;
    use crate::session::AiSession;
    use engine::ai_support::{ActionMetadata, AiDecisionContext, CandidateAction, TacticalClass};
    use engine::game::zones::create_object;
    use engine::types::ability::{
        AbilityDefinition, AbilityKind, ControllerRef, Effect, TargetFilter, TypedFilter,
    };
    use engine::types::card_type::{CardType, CoreType};
    use engine::types::game_state::{GameState, WaitingFor};
    use engine::types::identifiers::{CardId, ObjectId};
    use engine::types::keywords::Keyword;
    use engine::types::player::PlayerId;
    use engine::types::zones::Zone;
    use std::sync::Arc;

    const AI: PlayerId = PlayerId(0);
    const OPP: PlayerId = PlayerId(1);

    fn control_features_with_sweepers(sweeper_count: u32, commitment: f32) -> DeckFeatures {
        DeckFeatures {
            control: ControlFeature {
                sweeper_count,
                commitment,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn context_with_features(features: DeckFeatures) -> (AiContext, AiConfig) {
        let config = AiConfig::default();
        let mut session = AiSession::empty();
        session.features.insert(AI, features);
        let mut context = AiContext::empty(&config.weights);
        context.session = Arc::new(session);
        context.player = AI;
        (context, config)
    }

    fn cast_candidate(object_id: ObjectId, card_id: CardId) -> CandidateAction {
        CandidateAction {
            action: GameAction::CastSpell {
                object_id,
                card_id,
                targets: Vec::new(),

                payment_mode: CastPaymentMode::Auto,
            },
            metadata: ActionMetadata::for_actor(Some(AI), TacticalClass::Spell),
        }
    }

    fn priority_decision() -> AiDecisionContext {
        AiDecisionContext {
            waiting_for: WaitingFor::Priority { player: AI },
            candidates: Vec::new(),
        }
    }

    fn make_sweeper_spell(state: &mut GameState, idx: u64) -> ObjectId {
        let oid = create_object(
            state,
            CardId(6000 + idx),
            AI,
            format!("Wrath {idx}"),
            Zone::Hand,
        );
        let obj = state.objects.get_mut(&oid).unwrap();
        obj.card_types = CardType {
            supertypes: Vec::new(),
            core_types: vec![CoreType::Sorcery],
            subtypes: Vec::new(),
        };
        Arc::make_mut(&mut obj.abilities).push(AbilityDefinition::new(
            AbilityKind::Spell,
            Effect::DestroyAll {
                target: TargetFilter::Any,
                cant_regenerate: false,
            },
        ));
        oid
    }

    fn make_opponent_creature(state: &mut GameState, idx: u64) -> ObjectId {
        let oid = create_object(
            state,
            CardId(7000 + idx),
            OPP,
            format!("Opp Creature {idx}"),
            Zone::Battlefield,
        );
        let obj = state.objects.get_mut(&oid).unwrap();
        obj.card_types = CardType {
            supertypes: Vec::new(),
            core_types: vec![CoreType::Creature],
            subtypes: Vec::new(),
        };
        obj.power = Some(2);
        obj.toughness = Some(2);
        oid
    }

    fn make_non_sweeper_spell(state: &mut GameState, idx: u64) -> ObjectId {
        let oid = create_object(
            state,
            CardId(8000 + idx),
            AI,
            format!("Non-sweeper {idx}"),
            Zone::Hand,
        );
        let obj = state.objects.get_mut(&oid).unwrap();
        obj.card_types = CardType {
            supertypes: Vec::new(),
            core_types: vec![CoreType::Sorcery],
            subtypes: Vec::new(),
        };
        Arc::make_mut(&mut obj.abilities).push(AbilityDefinition::new(
            AbilityKind::Spell,
            Effect::Draw {
                count: engine::types::ability::QuantityExpr::Fixed { value: 2 },
                target: engine::types::ability::TargetFilter::Controller,
            },
        ));
        oid
    }

    #[test]
    fn activation_opts_out_with_zero_sweepers() {
        let features = control_features_with_sweepers(0, 0.8);
        let state = GameState::new_two_player(42);
        assert!(SweeperTimingPolicy
            .activation(&features, &state, AI)
            .is_none());
    }

    #[test]
    fn activation_opts_out_with_low_commitment() {
        let features = control_features_with_sweepers(3, 0.1);
        let state = GameState::new_two_player(42);
        assert!(SweeperTimingPolicy
            .activation(&features, &state, AI)
            .is_none());
    }

    #[test]
    fn activation_active_with_sweepers_and_commitment() {
        let features = control_features_with_sweepers(3, 0.6);
        let state = GameState::new_two_player(42);
        assert!(SweeperTimingPolicy
            .activation(&features, &state, AI)
            .is_some());
    }

    fn make_creature_for(state: &mut GameState, owner: PlayerId, idx: u64) -> ObjectId {
        let oid = create_object(
            state,
            CardId(9000 + idx),
            owner,
            format!("Creature {idx}"),
            Zone::Battlefield,
        );
        let obj = state.objects.get_mut(&oid).unwrap();
        obj.card_types = CardType {
            supertypes: Vec::new(),
            core_types: vec![CoreType::Creature],
            subtypes: Vec::new(),
        };
        obj.power = Some(2);
        obj.toughness = Some(2);
        oid
    }

    /// The verdict for a four-player free-for-all with `per_opponent` creatures
    /// on each opponent's board and `own` on the AI's, casting `sweeper_target`
    /// as a `DestroyAll` filter.
    fn multiplayer_verdict(
        per_opponent: [u64; 3],
        own: u64,
        sweeper_target: TargetFilter,
    ) -> (f64, String) {
        let mut state = GameState::new(engine::types::format::FormatConfig::free_for_all(), 4, 42);
        let mut idx = 0;
        for (seat, &n) in per_opponent.iter().enumerate() {
            for _ in 0..n {
                make_creature_for(&mut state, PlayerId(seat as u8 + 1), idx);
                idx += 1;
            }
        }
        for _ in 0..own {
            make_creature_for(&mut state, AI, idx);
            idx += 1;
        }
        let sweeper_id = make_sweeper_spell(&mut state, 0);
        let sweeper = state.objects.get_mut(&sweeper_id).unwrap();
        Arc::make_mut(&mut sweeper.abilities)[0] = AbilityDefinition::new(
            AbilityKind::Spell,
            Effect::DestroyAll {
                target: sweeper_target,
                cant_regenerate: false,
            },
        );
        let candidate = cast_candidate(sweeper_id, CardId(6000));
        let decision = priority_decision();
        let (context, config) = context_with_features(control_features_with_sweepers(3, 0.8));
        let ctx = PolicyContext {
            state: &state,
            decision: &decision,
            candidate: &candidate,
            ai_player: AI,
            config: &config,
            context: &context,
            cast_facts: None,
            search_depth: crate::policies::context::SearchDepth::Root,
        };
        match SweeperTimingPolicy.verdict(&ctx) {
            PolicyVerdict::Score { delta, reason } => (delta, reason.kind.to_string()),
            PolicyVerdict::Reject { .. } => panic!("unexpected Reject"),
        }
    }

    #[test]
    fn free_for_all_threshold_applies_to_the_leading_opponent() {
        // One creature each across three opponents meets the threshold in
        // total, but no single opponent has a board worth a wipe.
        assert_eq!(
            multiplayer_verdict([1, 1, 1], 0, TargetFilter::Any).1,
            "sweeper_premature"
        );
        // One opponent with three creatures against an empty board: timely.
        let (delta, kind) = multiplayer_verdict([3, 1, 0], 0, TargetFilter::Any);
        assert_eq!(kind, "sweeper_timely");
        assert!(delta > 0.0);
    }

    #[test]
    fn free_for_all_wipe_that_costs_the_caster_as_much_is_premature() {
        assert_eq!(
            multiplayer_verdict([3, 2, 2], 3, TargetFilter::Any).1,
            "sweeper_premature"
        );
        assert_eq!(
            multiplayer_verdict([4, 2, 2], 3, TargetFilter::Any).1,
            "sweeper_timely"
        );
    }

    #[test]
    fn free_for_all_one_sided_wipe_does_not_count_the_casters_creatures() {
        // "Destroy all creatures your opponents control" costs the caster
        // nothing, so its own board doesn't make the wipe premature.
        let opponents_creatures =
            TargetFilter::Typed(TypedFilter::creature().controller(ControllerRef::Opponent));
        assert_eq!(
            multiplayer_verdict([3, 2, 2], 3, opponents_creatures).1,
            "sweeper_timely"
        );
    }

    #[test]
    fn premature_penalty_under_threshold() {
        // 2 opposing creatures < 3 → premature.
        let mut state = GameState::new_two_player(42);
        make_opponent_creature(&mut state, 0);
        make_opponent_creature(&mut state, 1);

        let sweeper_id = make_sweeper_spell(&mut state, 0);
        let candidate = cast_candidate(sweeper_id, CardId(6000));
        let decision = priority_decision();
        let (context, config) = context_with_features(control_features_with_sweepers(3, 0.8));
        let ctx = PolicyContext {
            state: &state,
            decision: &decision,
            candidate: &candidate,
            ai_player: AI,
            config: &config,
            context: &context,
            cast_facts: None,
            search_depth: crate::policies::context::SearchDepth::Root,
        };

        let verdict = SweeperTimingPolicy.verdict(&ctx);
        match verdict {
            PolicyVerdict::Score { delta, reason } => {
                assert_eq!(reason.kind, "sweeper_premature");
                assert!(delta < 0.0, "expected negative delta, got {delta}");
            }
            PolicyVerdict::Reject { .. } => panic!("unexpected Reject"),
        }
    }

    #[test]
    fn timely_bonus_at_threshold() {
        // 3 opposing creatures ≥ 3 → timely.
        let mut state = GameState::new_two_player(42);
        for i in 0..3 {
            make_opponent_creature(&mut state, i);
        }

        let sweeper_id = make_sweeper_spell(&mut state, 0);
        let candidate = cast_candidate(sweeper_id, CardId(6000));
        let decision = priority_decision();
        let (context, config) = context_with_features(control_features_with_sweepers(3, 0.8));
        let ctx = PolicyContext {
            state: &state,
            decision: &decision,
            candidate: &candidate,
            ai_player: AI,
            config: &config,
            context: &context,
            cast_facts: None,
            search_depth: crate::policies::context::SearchDepth::Root,
        };

        let verdict = SweeperTimingPolicy.verdict(&ctx);
        match verdict {
            PolicyVerdict::Score { delta, reason } => {
                assert_eq!(reason.kind, "sweeper_timely");
                assert!(delta > 0.0, "expected positive delta, got {delta}");
            }
            PolicyVerdict::Reject { .. } => panic!("unexpected Reject"),
        }
    }

    #[test]
    fn non_sweeper_spell_is_neutral() {
        let mut state = GameState::new_two_player(42);
        for i in 0..5 {
            make_opponent_creature(&mut state, i);
        }

        let spell_id = make_non_sweeper_spell(&mut state, 0);
        let candidate = cast_candidate(spell_id, CardId(8000));
        let decision = priority_decision();
        let (context, config) = context_with_features(control_features_with_sweepers(3, 0.8));
        let ctx = PolicyContext {
            state: &state,
            decision: &decision,
            candidate: &candidate,
            ai_player: AI,
            config: &config,
            context: &context,
            cast_facts: None,
            search_depth: crate::policies::context::SearchDepth::Root,
        };

        let verdict = SweeperTimingPolicy.verdict(&ctx);
        match verdict {
            PolicyVerdict::Score { delta, reason } => {
                assert_eq!(reason.kind, "sweeper_timing_na");
                assert_eq!(delta, 0.0);
            }
            PolicyVerdict::Reject { .. } => panic!("unexpected Reject"),
        }
    }

    /// CR 702.94a + CR 603.11: the reported gap — the AI declined every miracle
    /// because `CastSpellAsMiracle` under `WaitingFor::MiracleReveal` routed to
    /// the activation bucket, so no cast policy scored it and the accept
    /// candidate carried no signal at all against the decline.
    ///
    /// Runs the accept and the decline through `PolicyRegistry` (the same
    /// `classify` → `by_kind` path production uses) with only this policy
    /// registered, so the assertion is about routing + the widened gate rather
    /// than the sum of thirty unrelated policies. Revert-failing on either half:
    /// without the routing change the accept never reaches `SweeperTiming`;
    /// without the widened gate `candidate_sweeper` rejects the miracle
    /// action and returns `sweeper_timing_na`.
    #[test]
    fn miracle_accept_outscores_decline_for_affordable_sweeper_with_opposing_creatures() {
        use crate::policies::registry::PolicyRegistry;
        use engine::types::mana::ManaCost;

        let mut state = GameState::new_two_player(42);
        for i in 0..3 {
            make_opponent_creature(&mut state, i);
        }
        let sweeper_id = make_sweeper_spell(&mut state, 0);
        {
            let obj = state.objects.get_mut(&sweeper_id).unwrap();
            obj.mana_cost = ManaCost::generic(6);
            // CR 702.94a: "Miracle {cost}" — the alternative cost this cast pays.
            obj.keywords.push(Keyword::Miracle(ManaCost::generic(1)));
        }

        let accept = CandidateAction {
            action: GameAction::CastSpellAsMiracle {
                object_id: sweeper_id,
                card_id: CardId(6000),
                payment_mode: CastPaymentMode::Auto,
            },
            metadata: ActionMetadata::for_actor(Some(AI), TacticalClass::Spell),
        };
        let decline = CandidateAction {
            action: GameAction::DecideOptionalEffect { accept: false },
            metadata: ActionMetadata::for_actor(Some(AI), TacticalClass::Pass),
        };
        let decision = AiDecisionContext {
            waiting_for: WaitingFor::MiracleReveal {
                player: AI,
                object_id: sweeper_id,
                cost: ManaCost::generic(1),
            },
            candidates: vec![accept.clone(), decline.clone()],
        };
        let (context, config) = context_with_features(control_features_with_sweepers(3, 0.8));
        let registry = PolicyRegistry::for_tests(vec![Box::new(SweeperTimingPolicy)]);

        let score = |candidate: &CandidateAction| {
            registry.score(&PolicyContext {
                state: &state,
                decision: &decision,
                candidate,
                ai_player: AI,
                config: &config,
                context: &context,
                cast_facts: crate::cast_facts::cast_facts_for_action(&state, &candidate.action, AI),
                search_depth: crate::policies::context::SearchDepth::Root,
            })
        };

        let accept_score = score(&accept);
        let decline_score = score(&decline);
        assert!(
            accept_score > decline_score,
            "miracle accept ({accept_score}) must outscore the decline ({decline_score})"
        );
        assert!(
            accept_score > 0.0,
            "the sweeper's timely bonus must actually reach the accept candidate"
        );
    }

    /// CR 118.9 + CR 107.3b: a free cast pays no mana, so the two policies that
    /// price a candidate off its PRINTED mana value must stay silent for it.
    /// Both still match `GameAction::CastSpell` literally and go neutral now
    /// that cast-family actions reach the `CastSpell` policy population; this
    /// pins that, so widening either of them without first consulting
    /// `CastFacts::cost_mode` breaks here rather than in a duel.
    ///
    /// Non-vacuity: the identical fixture with a plain `CastSpell` of the same
    /// card produces a non-zero verdict from each policy, so the zeros below are
    /// the free-cast variant's doing and not a dead fixture. Lives here rather
    /// than in either policy's own file so neither is touched by this step.
    #[test]
    fn free_cast_is_not_mana_priced() {
        use crate::policies::hold_mana_up::HoldManaUpForInteractionPolicy;
        use crate::policies::mana_efficiency::ManaEfficiencyPolicy;
        use engine::types::identifiers::CardId;
        use engine::types::mana::ManaCost;
        use engine::types::phase::Phase;

        let mut state = GameState::new_two_player(42);
        state.phase = Phase::PreCombatMain;
        // Two untapped lands: mana IS available, so the pricing arms are live.
        for idx in 0..2u64 {
            let land = create_object(
                &mut state,
                CardId(3100 + idx),
                AI,
                format!("Forest {idx}"),
                Zone::Battlefield,
            );
            let obj = state.objects.get_mut(&land).unwrap();
            obj.card_types = CardType {
                supertypes: Vec::new(),
                core_types: vec![CoreType::Land],
                subtypes: Vec::new(),
            };
        }
        // An instant in hand gives `hold_mana_up` something to hold up for.
        let instant = create_object(
            &mut state,
            CardId(3200),
            AI,
            "Counterspell".to_string(),
            Zone::Hand,
        );
        {
            let obj = state.objects.get_mut(&instant).unwrap();
            obj.card_types = CardType {
                supertypes: Vec::new(),
                core_types: vec![CoreType::Instant],
                subtypes: Vec::new(),
            };
            obj.mana_cost = ManaCost::generic(2);
        }
        let spell = create_object(
            &mut state,
            CardId(3300),
            AI,
            "Five Drop".to_string(),
            Zone::Hand,
        );
        {
            let obj = state.objects.get_mut(&spell).unwrap();
            obj.card_types = CardType {
                supertypes: Vec::new(),
                core_types: vec![CoreType::Sorcery],
                subtypes: Vec::new(),
            };
            obj.mana_cost = ManaCost::generic(5);
        }

        let (context, config) = context_with_features(DeckFeatures::default());
        let decision = priority_decision();

        let deltas = |action: GameAction| {
            let candidate = CandidateAction {
                metadata: ActionMetadata::for_actor(Some(AI), TacticalClass::Spell),
                action,
            };
            let ctx = PolicyContext {
                state: &state,
                decision: &decision,
                candidate: &candidate,
                ai_player: AI,
                config: &config,
                context: &context,
                cast_facts: crate::cast_facts::cast_facts_for_action(&state, &candidate.action, AI),
                search_depth: crate::policies::context::SearchDepth::Root,
            };
            let unwrap = |verdict: PolicyVerdict| match verdict {
                PolicyVerdict::Score { delta, .. } => delta,
                PolicyVerdict::Reject { .. } => panic!("unexpected Reject"),
            };
            (
                unwrap(HoldManaUpForInteractionPolicy.verdict(&ctx)),
                unwrap(ManaEfficiencyPolicy.verdict(&ctx)),
            )
        };

        let (printed_hold, printed_efficiency) = deltas(GameAction::CastSpell {
            object_id: spell,
            card_id: CardId(3300),
            targets: Vec::new(),
            payment_mode: CastPaymentMode::Auto,
        });
        assert!(
            printed_hold != 0.0 && printed_efficiency != 0.0,
            "control: a printed-cost cast of the same card must be priced \
             (hold_mana_up {printed_hold}, mana_efficiency {printed_efficiency})"
        );

        let (free_hold, free_efficiency) = deltas(GameAction::CastSpellForFree {
            object_id: spell,
            card_id: CardId(3300),
            source_id: ObjectId(1),
            payment_mode: CastPaymentMode::Auto,
        });
        assert_eq!(
            free_hold, 0.0,
            "free cast must not be priced by hold_mana_up"
        );
        assert_eq!(
            free_efficiency, 0.0,
            "free cast must not be priced by mana_efficiency"
        );
    }
}
