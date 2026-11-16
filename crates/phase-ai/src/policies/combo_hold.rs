//! Hold a combo's payoff until the rest of the combo is in place.
//!
//! A seat that rehearsed a combo and won with it ([`crate::combo::plan`]) is
//! putting it together. Its payoff (Walking Ballista behind a mana loop) and
//! the combo's own cards that are used up as they are cast score well alone:
//! Ballista for X=1 is a fine creature. Cast that way it is gone when the rest
//! arrives, and the loop has nothing to spend its mana on. This policy rejects
//! such a cast while the combo is still missing a card the seat can find.
//! Once every card is in place it says nothing: the certified line plays them.
//! Once a missing card can no longer be found (it was exiled, or the library
//! ran out of it), the combo is not pursued and the card is free again.
//!
//! A rejection, not a score: at an ordinary root decision the search weighs a
//! policy score at a tenth of the position it leads to, and a small creature
//! on the board outweighs any score in the strong band there.

use engine::types::actions::GameAction;
use engine::types::game_state::GameState;
use engine::types::player::PlayerId;

use crate::features::DeckFeatures;

use super::context::PolicyContext;
use super::registry::{DecisionKind, PolicyId, PolicyReason, PolicyVerdict, TacticalPolicy};

pub struct ComboHoldPolicy;

impl TacticalPolicy for ComboHoldPolicy {
    fn id(&self) -> PolicyId {
        PolicyId::ComboHold
    }

    fn decision_kinds(&self) -> &'static [DecisionKind] {
        &[DecisionKind::CastSpell]
    }

    fn activation(
        &self,
        _features: &DeckFeatures,
        _state: &GameState,
        _player: PlayerId,
    ) -> Option<f32> {
        // Whether there is a combo to hold for is the session's to say, in
        // `verdict`; a seat with none gets a neutral verdict there.
        Some(1.0)
    }

    fn verdict(&self, ctx: &PolicyContext<'_>) -> PolicyVerdict {
        let GameAction::CastSpell { object_id, .. } = &ctx.candidate.action else {
            return PolicyVerdict::neutral(PolicyReason::new("combo_hold_not_a_cast"));
        };
        let Some(object) = ctx.state.objects.get(object_id) else {
            return PolicyVerdict::neutral(PolicyReason::new("combo_hold_not_a_cast"));
        };
        let hold =
            crate::combo::plan::cards_to_hold(ctx.state, &ctx.context.session, ctx.ai_player);
        if hold.contains(&object.name.to_lowercase()) {
            PolicyVerdict::reject(PolicyReason::new("combo_card_held"))
        } else {
            PolicyVerdict::neutral(PolicyReason::new("combo_hold_nothing_held"))
        }
    }
}
