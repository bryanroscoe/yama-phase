//! Land Animation Timing Policy
//!
//! Evaluates when to animate man-lands like Lumbering Falls. Prevents the AI from
//! animating lands every turn regardless of strategic value, considering mana needs,
//! color requirements, and combat value.

use engine::game::game_object;
use engine::types::ability::{Effect, ManaProduction};
use engine::types::actions::GameAction;
use engine::types::card_type::CoreType;
use engine::types::game_state::GameState;
use engine::types::identifiers::ObjectId;
use engine::types::phase::Phase;
use engine::types::player::PlayerId;

use super::activation::turn_only;
use super::context::PolicyContext;
use super::registry::{DecisionKind, PolicyId, PolicyReason, PolicyVerdict, TacticalPolicy};
use crate::features::DeckFeatures;

/// Penalty for animating a land when mana is needed for other spells.
const MANA_NEEDED_PENALTY: f64 = -2.0;

/// Penalty for animating a land that ends up tapped (no combat value). Sits at
/// the bottom of the critical band (`-CRITICAL_MAX`) — the strongest finite
/// discouragement the score contract allows. Issue #5473: this was a raw -100.0
/// sentinel that bypassed the band helpers and tripped the registry's
/// critical-band assert once scaled by `activation` (turn_only, up to 1.3x).
///
/// Note: pinned at the critical ceiling, this branch is inert to `activation()`
/// tuning — `-CRITICAL_MAX × any activation` re-bands back to `-CRITICAL_MAX`.
const TAPPED_LAND_PENALTY: f64 = -super::registry::CRITICAL_MAX;

/// Bonus for animating when sufficient alternative mana sources exist.
const SUFFICIENT_MANA_BONUS: f64 = 0.3;

/// Penalty for animating a land outside any window in which it could attack or
/// block this turn. Same critical floor as [`TAPPED_LAND_PENALTY`]: the
/// activation buys a creature that can't fight before it stops being one.
const NO_COMBAT_WINDOW_PENALTY: f64 = -super::registry::CRITICAL_MAX;

pub struct LandAnimationPolicy;

impl TacticalPolicy for LandAnimationPolicy {
    fn id(&self) -> PolicyId {
        PolicyId::LandAnimation
    }

    fn decision_kinds(&self) -> &'static [DecisionKind] {
        &[DecisionKind::ActivateAbility]
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
        let GameAction::ActivateAbility {
            source_id,
            ability_index,
        } = &ctx.candidate.action
        else {
            return PolicyVerdict::Score {
                delta: 0.0,
                reason: PolicyReason::new("land_animation_na"),
            };
        };

        // Get the ability definition
        let Some(obj) = ctx.state.objects.get(source_id) else {
            return PolicyVerdict::Score {
                delta: 0.0,
                reason: PolicyReason::new("land_animation_na"),
            };
        };

        // Check if this is a land
        if !obj.card_types.core_types.contains(&CoreType::Land) {
            return PolicyVerdict::Score {
                delta: 0.0,
                reason: PolicyReason::new("land_animation_not_land"),
            };
        }

        let Some(ability_def) = obj.abilities.get(*ability_index) else {
            return PolicyVerdict::Score {
                delta: 0.0,
                reason: PolicyReason::new("land_animation_na"),
            };
        };

        if !crate::manland::animates_source(ability_def) {
            return PolicyVerdict::Score {
                delta: 0.0,
                reason: PolicyReason::new("land_animation_not_animation"),
            };
        }

        let mut delta = 0.0;

        // CR 508.1a / CR 509.1a: an animated land that ends up tapped can
        // neither attack nor block this turn, so animating it has no combat
        // value. This is the Shambling Vent failure mode — the AI taps the
        // manland for mana to help pay its own {1}{W}{B} animation cost and
        // turns it into a useless tapped creature. Strongly disprefer any
        // activation that leaves the source tapped.
        if crate::manland::activation_leaves_source_tapped(
            ctx.state,
            ctx.ai_player,
            *source_id,
            *ability_index,
            ability_def,
        ) {
            // Route the critical penalty through the band helper (CR-equivalent
            // score contract) rather than a raw Score literal so the delta stays
            // clamped to the critical band before `activation` scaling.
            return PolicyVerdict::critical(
                TAPPED_LAND_PENALTY,
                PolicyReason::new("land_animation_tapped"),
            );
        }

        // CR 508.1a + CR 302.6 + CR 509.1a: a man-land's animation lasts until
        // end of turn, so it only pays off if the creature can attack or block
        // this turn. The deck lab saw the AI animate Lumbering Falls on 152 of
        // its own turns and attack with it on 40, spending {2}{G}{U} each time.
        if !has_combat_window(ctx.state, ctx.ai_player, obj) {
            return PolicyVerdict::critical(
                NO_COMBAT_WINDOW_PENALTY,
                PolicyReason::new("land_animation_no_combat_window"),
            );
        }

        // Animating a land that is already a creature this turn buys nothing
        // (the lab saw Lumbering Falls animated three times in one turn).
        if obj.card_types.core_types.contains(&CoreType::Creature) {
            return PolicyVerdict::critical(
                NO_COMBAT_WINDOW_PENALTY,
                PolicyReason::new("land_animation_already_creature"),
            );
        }

        // CR 508.1: on its own turn the AI animates a man-land to attack with
        // it. Most of the lab's wasted animations came before combat and were
        // followed by an attack without the land, or none. Ask the combat AI
        // whether it would attack with the animated body on this board.
        if ctx.state.active_player == ctx.ai_player
            && !animated_land_would_attack(ctx.state, ctx.ai_player, *source_id, ability_def)
        {
            return PolicyVerdict::critical(
                NO_COMBAT_WINDOW_PENALTY,
                PolicyReason::new("land_animation_would_not_attack"),
            );
        }

        // Check if this is the only source of a critical color
        let is_critical_color_source = is_only_source_of_color(ctx, *source_id);
        if is_critical_color_source {
            delta += MANA_NEEDED_PENALTY;
        }

        // Check if mana is needed for spells in hand
        let mana_needed = mana_needed_in_hand(ctx);
        if mana_needed {
            delta += MANA_NEEDED_PENALTY;
        }

        // Bonus if sufficient alternative mana sources exist
        let sufficient_mana = has_sufficient_mana_sources(ctx, *source_id);
        if sufficient_mana {
            delta += SUFFICIENT_MANA_BONUS;
        }

        PolicyVerdict::Score {
            delta,
            reason: PolicyReason::new("land_animation_score"),
        }
    }
}

/// Whether a land animated now could still fight this turn.
///
/// - On the AI's own turn it can attack only if attackers haven't been declared
///   yet (CR 508.1) and the AI has controlled the land continuously since the
///   turn began (CR 302.6: the engine's `summoning_sick` flag, which it keeps
///   for every permanent).
/// - On an opponent's turn it can only block, and blocking needs a creature
///   attacking the AI: the window is the declare-attackers step once attacks
///   are on the board (CR 509.1a).
///
/// "Could" is not "will": the combat AI still decides whether to attack. This
/// only rules out windows where no attack or block is possible at all.
fn has_combat_window(
    state: &GameState,
    ai_player: PlayerId,
    land: &game_object::GameObject,
) -> bool {
    if state.active_player == ai_player {
        let before_attacks = matches!(
            state.phase,
            Phase::Untap | Phase::Upkeep | Phase::Draw | Phase::PreCombatMain | Phase::BeginCombat
        );
        return before_attacks && !land.summoning_sick;
    }
    state.phase == Phase::DeclareAttackers
        && state.combat.as_ref().is_some_and(|combat| {
            combat
                .attackers
                .iter()
                .any(|attacker| attacker.defending_player == ai_player)
        })
}

/// CR 508.1: whether the combat AI would attack with `land_id` once animated,
/// judged on the current board. The animation is previewed on a copy of the
/// state: the land becomes a creature with the animated body's power,
/// toughness and keywords. A body whose size the activation sets from X
/// (Lair of the Hydra) is sized by the mana the AI could put into X: its other
/// untapped lands, less the cost's colored symbols. An ability whose body
/// can't be read is let through.
fn animated_land_would_attack(
    state: &GameState,
    ai_player: PlayerId,
    land_id: ObjectId,
    ability: &engine::types::ability::AbilityDefinition,
) -> bool {
    let Some(body) = crate::manland::extract_body(ability) else {
        return true;
    };
    let (power, toughness) = if body.power > 0 || body.toughness > 0 {
        (body.power, body.toughness)
    } else {
        let x = x_mana_available(state, ai_player, land_id, ability);
        (x, x)
    };
    if power <= 0 {
        return false;
    }
    let mut preview = state.clone();
    let Some(land) = preview.objects.get_mut(&land_id) else {
        return true;
    };
    if !land.card_types.core_types.contains(&CoreType::Creature) {
        land.card_types.core_types.push(CoreType::Creature);
    }
    land.power = Some(power);
    land.toughness = Some(toughness);
    for keyword in &body.keywords {
        if !land.keywords.contains(keyword) {
            land.keywords.push(keyword.clone());
        }
    }
    crate::combat_ai::choose_attackers(&preview, ai_player).contains(&land_id)
}

/// Mana the AI could spend on X for `ability`: its untapped lands other than
/// `land_id`, less the colored symbols in the cost (at least 0).
fn x_mana_available(
    state: &GameState,
    ai_player: PlayerId,
    land_id: ObjectId,
    ability: &engine::types::ability::AbilityDefinition,
) -> i32 {
    let other_untapped = state
        .battlefield
        .iter()
        .filter(|&&id| id != land_id)
        .filter_map(|id| state.objects.get(id))
        .filter(|o| {
            o.controller == ai_player
                && o.card_types.core_types.contains(&CoreType::Land)
                && !o.tapped
        })
        .count() as i32;
    let colored = match ability.cost.as_ref() {
        Some(engine::types::ability::AbilityCost::Mana {
            cost: engine::types::mana::ManaCost::Cost { shards, .. },
        }) => shards
            .iter()
            .filter(|s| !matches!(s, engine::types::mana::ManaCostShard::X))
            .count() as i32,
        _ => 0,
    };
    (other_untapped - colored).max(0)
}

/// Check if this land is the only source of a critical color for the AI.
fn is_only_source_of_color(ctx: &PolicyContext<'_>, land_id: ObjectId) -> bool {
    let Some(land) = ctx.state.objects.get(&land_id) else {
        return false;
    };

    // Get colors this land can produce
    let land_colors = colors_produced_by_land(land);

    // For each color, check if this is the only source
    for color in land_colors {
        let other_sources = ctx
            .state
            .battlefield
            .iter()
            .filter(|&&id| {
                id != land_id && {
                    let Some(obj) = ctx.state.objects.get(&id) else {
                        return false;
                    };
                    obj.controller == ctx.ai_player
                        && obj.card_types.core_types.contains(&CoreType::Land)
                        && !obj.tapped
                        && colors_produced_by_land(obj).contains(&color)
                }
            })
            .count();

        if other_sources == 0 {
            return true;
        }
    }

    false
}

/// Get the colors a land can produce.
fn colors_produced_by_land(land: &game_object::GameObject) -> Vec<engine::types::mana::ManaColor> {
    let mut colors = Vec::new();
    for ability in land.abilities.iter() {
        if let Effect::Mana { produced, .. } = &*ability.effect {
            match produced {
                ManaProduction::Fixed {
                    colors: produced_colors,
                    ..
                } => {
                    colors.extend(produced_colors.clone());
                }
                ManaProduction::Mixed {
                    colors: produced_colors,
                    ..
                } => {
                    colors.extend(produced_colors.clone());
                }
                ManaProduction::AnyOneColor { color_options, .. } => {
                    colors.extend(color_options.clone());
                }
                ManaProduction::AnyCombination { color_options, .. } => {
                    colors.extend(color_options.clone());
                }
                ManaProduction::ChosenColor {
                    fixed_alternative, ..
                } => {
                    if let Some(c) = land.chosen_color() {
                        colors.push(c);
                    }
                    if let Some(c) = fixed_alternative {
                        colors.push(*c);
                    }
                }
                // CR 202.2c: Omnath, Locus of All — colors come from a target
                // object resolved at trigger time, not statically predictable
                // for land-animation color preview. Contribute nothing.
                ManaProduction::AnyCombinationOfObjectColors { .. } => {}
                _ => {}
            }
        }
    }
    colors
}

/// Check if the AI needs mana for spells in hand.
fn mana_needed_in_hand(ctx: &PolicyContext<'_>) -> bool {
    // Check if AI has spells in hand that require mana
    let has_spells = ctx.state.players[ctx.ai_player.0 as usize]
        .hand
        .iter()
        .any(|&object_id| {
            let Some(obj) = ctx.state.objects.get(&object_id) else {
                return false;
            };
            // Simple heuristic: if object has a mana cost, AI needs mana
            obj.mana_cost.mana_value() > 0
        });

    // Check if AI has untapped mana sources
    let has_untapped_mana = ctx.state.battlefield.iter().any(|&id| {
        let Some(obj) = ctx.state.objects.get(&id) else {
            return false;
        };
        obj.controller == ctx.ai_player
            && obj.card_types.core_types.contains(&CoreType::Land)
            && !obj.tapped
    });

    has_spells && !has_untapped_mana
}

/// Check if the AI has sufficient alternative mana sources.
fn has_sufficient_mana_sources(ctx: &PolicyContext<'_>, exclude_land: ObjectId) -> bool {
    let land_count = ctx
        .state
        .battlefield
        .iter()
        .filter(|&&id| {
            id != exclude_land && {
                let Some(obj) = ctx.state.objects.get(&id) else {
                    return false;
                };
                obj.controller == ctx.ai_player
                    && obj.card_types.core_types.contains(&CoreType::Land)
            }
        })
        .count();

    land_count >= 3 // Heuristic: need at least 3 other lands
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::config::AiConfig;
    use crate::context::AiContext;
    use engine::ai_support::{ActionMetadata, AiDecisionContext, CandidateAction, TacticalClass};
    use engine::game::zones::create_object;
    use engine::types::ability::{
        AbilityCost, AbilityDefinition, AbilityKind, ContinuousModification, PtValue, QuantityExpr,
        StaticDefinition, TargetFilter,
    };
    use engine::types::game_state::WaitingFor;
    use engine::types::identifiers::CardId;
    use engine::types::mana::{ManaColor, ManaCost, ManaCostShard};
    use engine::types::statics::StaticMode;
    use engine::types::zones::Zone;

    const AI: PlayerId = PlayerId(0);

    fn mana_effect(colors: Vec<ManaColor>) -> Effect {
        Effect::Mana {
            produced: ManaProduction::Fixed {
                colors,
                contribution: Default::default(),
            },
            restrictions: Vec::new(),
            grants: Vec::new(),
            expiry: None,
            target: None,
        }
    }

    fn animate_effect() -> Effect {
        Effect::Animate {
            power: Some(PtValue::Fixed(2)),
            toughness: Some(PtValue::Fixed(2)),
            types: vec!["Creature".to_string()],
            remove_types: Vec::new(),
            target: TargetFilter::SelfRef,
            keywords: Vec::new(),
        }
    }

    fn generic_creature_type_effect() -> Effect {
        Effect::GenericEffect {
            static_abilities: vec![StaticDefinition::new(StaticMode::Continuous).modifications(
                vec![ContinuousModification::AddType {
                    core_type: CoreType::Creature,
                }],
            )],
            duration: None,
            target: Some(TargetFilter::SelfRef),
            end_cost: None,
        }
    }

    fn land_with_ability(state: &mut GameState, ability: AbilityDefinition) -> ObjectId {
        let id = create_object(
            state,
            CardId(state.objects.len() as u64 + 1),
            AI,
            "Test Land".to_string(),
            Zone::Battlefield,
        );
        let obj = state.objects.get_mut(&id).unwrap();
        obj.card_types.core_types.push(CoreType::Land);
        Arc::make_mut(&mut obj.abilities).push(ability);
        // A land the AI has controlled since the turn began (CR 302.6).
        obj.summoning_sick = false;
        obj.entered_battlefield_turn = Some(0);
        state.turn_number = state.turn_number.max(2);
        id
    }

    /// A manland whose animation is affordable from three other lands, so the
    /// tapped-source rule doesn't fire and only the combat window decides.
    fn affordable_manland(state: &mut GameState) -> ObjectId {
        let source_id = land_with_ability(state, animate_ability_with_mana_cost());
        mana_land(state, ManaColor::White);
        mana_land(state, ManaColor::Black);
        mana_land(state, ManaColor::White);
        source_id
    }

    #[test]
    fn animation_before_own_combat_is_allowed() {
        let mut state = GameState::new_two_player(42);
        let source_id = affordable_manland(&mut state);
        state.active_player = AI;
        state.phase = Phase::PreCombatMain;
        assert_eq!(
            reason_kind(&policy_verdict(&state, source_id)),
            "land_animation_score"
        );
    }

    #[test]
    fn animation_after_own_attacks_is_penalized() {
        // CR 508.1: attackers are declared once; animating in the second main
        // phase buys a creature that can neither attack nor block.
        let mut state = GameState::new_two_player(42);
        let source_id = affordable_manland(&mut state);
        state.active_player = AI;
        state.phase = Phase::PostCombatMain;
        let verdict = policy_verdict(&state, source_id);
        assert_eq!(reason_kind(&verdict), "land_animation_no_combat_window");
        let PolicyVerdict::Score { delta, .. } = verdict else {
            panic!("expected score verdict");
        };
        assert_eq!(delta, NO_COMBAT_WINDOW_PENALTY);
    }

    #[test]
    fn animating_a_land_played_this_turn_is_penalized() {
        // CR 302.6: it hasn't been under the AI's control since the turn began.
        let mut state = GameState::new_two_player(42);
        let source_id = affordable_manland(&mut state);
        state.objects.get_mut(&source_id).unwrap().summoning_sick = true;
        state.active_player = AI;
        state.phase = Phase::PreCombatMain;
        assert_eq!(
            reason_kind(&policy_verdict(&state, source_id)),
            "land_animation_no_combat_window"
        );
    }

    #[test]
    fn animation_into_a_bigger_untapped_blocker_is_penalized() {
        // The combat AI won't attack a 2/2 into an untapped 6/6, so animating
        // the land precombat only spends the mana.
        let mut state = GameState::new_two_player(42);
        let source_id = affordable_manland(&mut state);
        state.active_player = AI;
        state.phase = Phase::PreCombatMain;
        let blocker = create_object(
            &mut state,
            CardId(901),
            PlayerId(1),
            "Big Blocker".to_string(),
            Zone::Battlefield,
        );
        {
            let b = state.objects.get_mut(&blocker).unwrap();
            b.card_types.core_types.push(CoreType::Creature);
            b.power = Some(6);
            b.toughness = Some(6);
            b.base_power = Some(6);
            b.base_toughness = Some(6);
            b.summoning_sick = false;
            b.entered_battlefield_turn = Some(0);
        }
        assert_eq!(
            reason_kind(&policy_verdict(&state, source_id)),
            "land_animation_would_not_attack"
        );
    }

    #[test]
    fn animating_a_land_that_is_already_a_creature_is_penalized() {
        let mut state = GameState::new_two_player(42);
        let source_id = affordable_manland(&mut state);
        state.active_player = AI;
        state.phase = Phase::PreCombatMain;
        state
            .objects
            .get_mut(&source_id)
            .unwrap()
            .card_types
            .core_types
            .push(CoreType::Creature);
        assert_eq!(
            reason_kind(&policy_verdict(&state, source_id)),
            "land_animation_already_creature"
        );
    }

    #[test]
    fn animation_on_opponents_turn_needs_an_attack_on_the_ai() {
        let mut state = GameState::new_two_player(42);
        let source_id = affordable_manland(&mut state);
        state.active_player = PlayerId(1);
        state.phase = Phase::End;
        assert_eq!(
            reason_kind(&policy_verdict(&state, source_id)),
            "land_animation_no_combat_window",
            "nothing to block in the opponent's end step"
        );

        // CR 509.1a: once a creature attacks the AI, an animated land can block.
        let attacker = create_object(
            &mut state,
            CardId(900),
            PlayerId(1),
            "Attacker".to_string(),
            Zone::Battlefield,
        );
        state.phase = Phase::DeclareAttackers;
        state.combat = Some(engine::game::combat::CombatState {
            attackers: vec![engine::game::combat::AttackerInfo::new(
                attacker,
                engine::game::combat::AttackTarget::Player(AI),
                AI,
            )],
            ..Default::default()
        });
        assert_eq!(
            reason_kind(&policy_verdict(&state, source_id)),
            "land_animation_score"
        );
    }

    fn policy_verdict(state: &GameState, source_id: ObjectId) -> PolicyVerdict {
        let decision = AiDecisionContext {
            waiting_for: WaitingFor::Priority { player: AI },
            candidates: Vec::new(),
        };
        let candidate = CandidateAction {
            action: GameAction::ActivateAbility {
                source_id,
                ability_index: 0,
            },
            metadata: ActionMetadata::for_actor(Some(AI), TacticalClass::Ability),
        };
        let config = AiConfig::default();
        let context = AiContext::empty(&config.weights);
        let ctx = PolicyContext {
            state,
            decision: &decision,
            candidate: &candidate,
            ai_player: AI,
            config: &config,
            context: &context,
            cast_facts: None,
            search_depth: crate::policies::context::SearchDepth::Root,
        };
        LandAnimationPolicy.verdict(&ctx)
    }

    fn assert_score(verdict: PolicyVerdict, expected_reason: &str) {
        let PolicyVerdict::Score { delta, reason } = verdict else {
            panic!("expected score verdict");
        };
        assert_eq!(delta, 0.0);
        assert_eq!(reason.kind, expected_reason);
    }

    fn reason_kind(verdict: &PolicyVerdict) -> &str {
        let PolicyVerdict::Score { reason, .. } = verdict else {
            panic!("expected score verdict");
        };
        reason.kind
    }

    /// {1}{W}{B} animation ability (mana value 3), mirroring Shambling Vent.
    fn animate_ability_with_mana_cost() -> AbilityDefinition {
        AbilityDefinition::new(AbilityKind::Activated, animate_effect()).cost(AbilityCost::Mana {
            cost: ManaCost::Cost {
                shards: vec![ManaCostShard::White, ManaCostShard::Black],
                generic: 1,
            },
        })
    }

    /// {1}{W}{B} animation via the `GenericEffect` + `AddType Creature` shape
    /// that real man-lands (Shambling Vent) actually use, not the synthetic
    /// `Effect::Animate`. Locks the production code path through the guard.
    fn generic_animate_ability_with_mana_cost() -> AbilityDefinition {
        AbilityDefinition::new(AbilityKind::Activated, generic_creature_type_effect()).cost(
            AbilityCost::Mana {
                cost: ManaCost::Cost {
                    shards: vec![ManaCostShard::White, ManaCostShard::Black],
                    generic: 1,
                },
            },
        )
    }

    /// An untapped basic-style land: `{T}: Add {color}`.
    fn mana_land(state: &mut GameState, color: ManaColor) -> ObjectId {
        land_with_ability(
            state,
            AbilityDefinition::new(AbilityKind::Activated, mana_effect(vec![color]))
                .cost(AbilityCost::Tap),
        )
    }

    #[test]
    fn animation_forcing_self_tap_for_mana_is_penalized() {
        // Manland is untapped but the AI has no other mana: paying {1}{W}{B}
        // would tap the manland itself, animating it into a tapped creature.
        let mut state = GameState::new_two_player(42);
        let source_id = land_with_ability(&mut state, animate_ability_with_mana_cost());

        let verdict = policy_verdict(&state, source_id);
        assert_eq!(reason_kind(&verdict), "land_animation_tapped");
        let PolicyVerdict::Score { delta, .. } = verdict else {
            panic!("expected score verdict");
        };
        assert_eq!(delta, TAPPED_LAND_PENALTY);
    }

    #[test]
    fn animation_with_sufficient_other_mana_is_not_tapped_penalized() {
        // W + B + a third source cover {1}{W}{B} without tapping the manland,
        // so it can animate and still attack/block this turn.
        let mut state = GameState::new_two_player(42);
        let source_id = land_with_ability(&mut state, animate_ability_with_mana_cost());
        mana_land(&mut state, ManaColor::White);
        mana_land(&mut state, ManaColor::Black);
        mana_land(&mut state, ManaColor::White);

        let verdict = policy_verdict(&state, source_id);
        assert_eq!(reason_kind(&verdict), "land_animation_score");
    }

    #[test]
    fn animation_color_starved_off_source_is_penalized() {
        // Three other untapped sources cover the *total* of {1}{W}{B}, but none
        // produces black — only the manland could, so the engine would tap it
        // for {B} and leave a tapped creature. A count-only check would miss
        // this; the color-aware engine solver catches it.
        let mut state = GameState::new_two_player(42);
        let source_id = land_with_ability(&mut state, animate_ability_with_mana_cost());
        mana_land(&mut state, ManaColor::White);
        mana_land(&mut state, ManaColor::White);
        mana_land(&mut state, ManaColor::Green);

        let verdict = policy_verdict(&state, source_id);
        assert_eq!(reason_kind(&verdict), "land_animation_tapped");
    }

    #[test]
    fn generic_effect_manland_self_tap_is_penalized() {
        // Real Shambling Vent shape (GenericEffect AddType Creature). With no
        // other mana, paying {1}{W}{B} forces tapping the manland → penalized.
        let mut state = GameState::new_two_player(42);
        let source_id = land_with_ability(&mut state, generic_animate_ability_with_mana_cost());

        let verdict = policy_verdict(&state, source_id);
        assert_eq!(reason_kind(&verdict), "land_animation_tapped");
    }

    #[test]
    fn already_tapped_manland_animation_is_penalized() {
        let mut state = GameState::new_two_player(42);
        let source_id = land_with_ability(&mut state, animate_ability_with_mana_cost());
        mana_land(&mut state, ManaColor::White);
        mana_land(&mut state, ManaColor::Black);
        mana_land(&mut state, ManaColor::White);
        state.objects.get_mut(&source_id).unwrap().tapped = true;

        let verdict = policy_verdict(&state, source_id);
        assert_eq!(reason_kind(&verdict), "land_animation_tapped");
    }

    #[test]
    fn mana_ability_on_land_is_not_animation() {
        let mut state = GameState::new_two_player(42);
        let source_id = land_with_ability(
            &mut state,
            AbilityDefinition::new(AbilityKind::Activated, mana_effect(vec![ManaColor::Green])),
        );

        assert_score(
            policy_verdict(&state, source_id),
            "land_animation_not_animation",
        );
    }

    #[test]
    fn ability_animates_land_walks_sub_ability_chain() {
        let mut ability =
            AbilityDefinition::new(AbilityKind::Activated, mana_effect(vec![ManaColor::Green]));
        ability.sub_ability = Some(Box::new(AbilityDefinition::new(
            AbilityKind::Activated,
            animate_effect(),
        )));

        assert!(crate::manland::animates_source(&ability));
    }

    #[test]
    fn ability_animates_land_detects_generic_creature_type_grant() {
        let ability =
            AbilityDefinition::new(AbilityKind::Activated, generic_creature_type_effect());

        assert!(crate::manland::animates_source(&ability));
    }

    #[test]
    fn colors_produced_by_land_handles_any_one_color() {
        let mut state = GameState::new_two_player(42);
        let source_id = land_with_ability(
            &mut state,
            AbilityDefinition::new(
                AbilityKind::Activated,
                Effect::Mana {
                    produced: ManaProduction::AnyOneColor {
                        count: QuantityExpr::Fixed { value: 1 },
                        color_options: vec![ManaColor::White, ManaColor::Blue],
                        contribution: Default::default(),
                    },
                    restrictions: Vec::new(),
                    grants: Vec::new(),
                    expiry: None,
                    target: None,
                },
            ),
        );

        let colors = colors_produced_by_land(state.objects.get(&source_id).unwrap());
        assert_eq!(colors, vec![ManaColor::White, ManaColor::Blue]);
    }
}
