//! A practice position: a copy of a game in which one player already has some of
//! their own cards out, for analysis that asks "what could I do if I had these".
//!
//! This is not a game action and never touches the game it is given. An AI uses
//! it to rehearse a combination of cards before it has drawn them, so it knows
//! which of its deck's combinations it can actually play.

use crate::game::zone_pipeline::{move_object, ZoneMoveRequest, ZoneMoveResult};
use crate::types::card_type::{CoreType, Supertype};
use crate::types::game_state::GameState;
use crate::types::identifiers::ObjectId;
use crate::types::player::PlayerId;
use crate::types::zones::Zone;

/// What a practice position holds beyond the game it was copied from.
#[derive(Debug, Clone, Copy)]
pub struct PracticeSetup<'a> {
    /// Cards put onto the battlefield untapped, as though they had entered on an
    /// earlier turn: nothing triggers on their entering.
    pub battlefield: &'a [ObjectId],
    /// The player's hand: these cards. Anything else in it is put into the
    /// library, so what the player happens to hold doesn't change what the
    /// position shows; a card that can't go there without a choice being made
    /// for it (a commander, CR 903.9b) stays in hand.
    pub hand: &'a [ObjectId],
    /// How many lands from the player's library are put onto the battlefield
    /// untapped beside them, one of each name before a second of any, basic
    /// lands first. A land that cannot enter without a choice is passed over.
    pub lands: usize,
}

/// A copy of `state` with `setup` applied for `player`, or `None` when it cannot
/// be built: a card is not `player`'s, is somewhere other than their library or
/// hand, or cannot enter without a choice being made for it (an "as this enters"
/// choice, an Aura with nothing chosen to enchant), or a card for the hand
/// cannot be put there.
///
/// Every move goes through the zone-change pipeline, so replacement effects on
/// entering apply. The events of the moves are dropped, which is what makes the
/// cards "already there": no enters trigger is put on the stack.
pub fn practice_position(
    state: &GameState,
    player: PlayerId,
    setup: &PracticeSetup<'_>,
) -> Option<GameState> {
    let mut copy = state.clone();
    let movable = |copy: &GameState, id: ObjectId| {
        copy.objects.get(&id).is_some_and(|object| {
            object.owner == player && matches!(object.zone, Zone::Library | Zone::Hand)
        })
    };
    if !setup
        .battlefield
        .iter()
        .chain(setup.hand)
        .all(|id| movable(&copy, *id))
    {
        return None;
    }

    for id in setup.battlefield {
        if !enter_ahead(&mut copy, *id) {
            return None;
        }
    }
    // A land that asks something as it enters (a land that may be paid for, one
    // that names a type) is passed over for the next: the position needs mana,
    // not that land. Each is tried on a copy, since a move that stops for a
    // choice leaves that choice pending.
    let mut placed = 0;
    for id in practice_lands(&copy, player, setup) {
        if placed == setup.lands {
            break;
        }
        let mut trial = copy.clone();
        if enter_ahead(&mut trial, id) {
            copy = trial;
            placed += 1;
        }
    }
    let mut events = Vec::new();
    let held: Vec<ObjectId> = copy
        .players
        .iter()
        .find(|seat| seat.id == player)
        .map(|seat| seat.hand.iter().copied().collect())
        .unwrap_or_default();
    for id in held {
        if setup.hand.contains(&id) {
            continue;
        }
        // Tried on a copy: a move that stops for a choice leaves it pending.
        let mut trial = copy.clone();
        let request = ZoneMoveRequest::effect(id, Zone::Library, id);
        if matches!(
            move_object(&mut trial, request, &mut events),
            ZoneMoveResult::Done
        ) {
            copy = trial;
        }
    }
    for id in setup.hand {
        if copy.objects.get(id)?.zone == Zone::Hand {
            continue;
        }
        let request = ZoneMoveRequest::effect(*id, Zone::Hand, *id);
        if !matches!(
            move_object(&mut copy, request, &mut events),
            ZoneMoveResult::Done
        ) || copy.objects.get(id)?.zone != Zone::Hand
        {
            return None;
        }
    }
    Some(copy)
}

/// Put `id` onto the battlefield of `copy` untapped, as though it had entered
/// on an earlier turn. `false` when the move stopped for a choice or a
/// replacement sent the card somewhere else; `copy` is then not to be used.
fn enter_ahead(copy: &mut GameState, id: ObjectId) -> bool {
    let mut events = Vec::new();
    let request = ZoneMoveRequest::effect(id, Zone::Battlefield, id);
    if !matches!(
        move_object(copy, request, &mut events),
        ZoneMoveResult::Done
    ) {
        return false;
    }
    let entered = copy.turn_number.saturating_sub(1);
    let Some(object) = copy.objects.get_mut(&id) else {
        return false;
    };
    if object.zone != Zone::Battlefield {
        return false;
    }
    object.entered_battlefield_turn = Some(entered);
    object.summoning_sick = false;
    object.tapped = false;
    crate::game::trigger_index::reindex_object_triggers(copy, id);
    true
}

/// The lands a practice position may take from `player`'s library, in the
/// order it takes them: one of each name before a second of any, basic lands
/// first, so a deck of several colors gets each of them. Cards the setup
/// already places are left alone.
fn practice_lands(state: &GameState, player: PlayerId, setup: &PracticeSetup<'_>) -> Vec<ObjectId> {
    let Some(seat) = state.players.iter().find(|seat| seat.id == player) else {
        return Vec::new();
    };
    // (copies of this name already listed, not basic, library position)
    let mut lands: Vec<((usize, bool, usize), ObjectId)> = Vec::new();
    let mut seen: Vec<(&str, usize)> = Vec::new();
    for (position, id) in seat.library.iter().enumerate() {
        let Some(object) = state.objects.get(id) else {
            continue;
        };
        if !object.card_types.core_types.contains(&CoreType::Land)
            || setup.battlefield.contains(id)
            || setup.hand.contains(id)
        {
            continue;
        }
        let basic = object.card_types.supertypes.contains(&Supertype::Basic);
        let copies = match seen.iter_mut().find(|(name, _)| *name == object.name) {
            Some((_, count)) => {
                *count += 1;
                *count - 1
            }
            None => {
                seen.push((object.name.as_str(), 1));
                0
            }
        };
        lands.push(((copies, !basic, position), *id));
    }
    lands.sort_unstable_by_key(|(key, _)| *key);
    lands.into_iter().map(|(_, id)| id).collect()
}
