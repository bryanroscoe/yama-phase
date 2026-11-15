//! The deck plan: what a seat has learned about its own combos, and what it is
//! looking for because of it.
//!
//! ## Learning (once per game)
//!
//! A catalog says which cards combine. It does not say whether this AI can win
//! with them. So the seat rehearses: on a copy of the game it puts a combo's
//! cards out as though earlier turns had played them
//! ([`engine::ai_support::practice_position`]), with a few of its own lands,
//! and runs the same search it would run in the real game
//! ([`super::certify`]). A combo is kept as a goal only if that search wins.
//!
//! A combo that makes an unbounded resource and stops (a mana loop with nothing
//! to spend it on) is rehearsed once more with the deck's X spells in hand. If
//! that wins, the X spell that did it is remembered as part of the goal.
//!
//! One combo is rehearsed a turn, at the seat's first decision in its own main
//! phase, up to [`MAX_REHEARSALS`] a game. A host that has rehearsed a deck
//! before can hand the results in ([`crate::session::AiSession::with_rehearsals`])
//! and none are run.
//!
//! ## Playing (every decision)
//!
//! [`wanted_cards`] names the cards of those combos the seat does not have yet
//! and could still find in its library. A search of the library takes one of
//! them ahead of any other card, and a spell that searches is worth more while
//! there is one to find (`policies/tutor.rs`).
//!
//! ## What it does not do yet
//!
//! - It does not shape mulligans, hold a piece back from a discard or a block,
//!   or count progress toward a combo in the position score.
//! - A card that is only in the graveyard or in exile counts as held, as it
//!   does for the live search, whether or not the combo can use it from there.
//! - The only payoff it looks for is an X spell.

use std::collections::HashMap;

use engine::ai_support::{practice_position, PracticeSetup};
use engine::game::game_object::GameObject;
use engine::types::card_type::CoreType;
use engine::types::game_state::GameState;
use engine::types::identifiers::ObjectId;
use engine::types::player::PlayerId;
use engine::types::zones::Zone;
use engine::util::Deadline;
use serde::{Deserialize, Serialize};

use crate::config::AiConfig;
use crate::session::AiSession;

use super::catalog::CatalogCombo;
use super::certify::{self, SearchOutcome};

/// Rehearsals one seat runs in one game.
const MAX_REHEARSALS: usize = 4;

/// The work one rehearsal search may spend. A line the live search can find is
/// found well inside this; a combo that needs more is one the seat could not
/// play in a real game either.
const REHEARSAL_WORK: usize = 2500;

/// Wall-clock budget of one rehearsal outside measurement mode.
const REHEARSAL_BUDGET_MS: u32 = 1500;

/// Lands a practice position has beyond the mana value of the cards it puts in
/// hand: enough for an activation or two.
const SPARE_LANDS: usize = 6;

/// Lands a practice position has for each card in hand that is cast for X,
/// whose mana value counts nothing: enough for an X of two.
const X_SPELL_LANDS: usize = 4;

/// The most lands a practice position has.
const MAX_LANDS: usize = 14;

/// X spells tried as the payoff of a combo that only makes a resource.
const MAX_PAYOFFS: usize = 6;

/// What rehearsing a combo showed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Rehearsal {
    /// The seat won the practice game. `with` names cards outside the combo
    /// that the win needed: the spell that spent what the combo made.
    Wins { with: Vec<String> },
    /// The seat did not find a win.
    NoWin,
}

/// What one seat has learned about its combos in this game.
#[derive(Debug, Clone, Default)]
pub(crate) struct DeckPlan {
    /// By the combo's catalog id.
    pub(crate) known: HashMap<String, Rehearsal>,
    /// The turn of the last rehearsal: one is run a turn.
    last_turn: Option<u32>,
    /// Rehearsals run in this game.
    run: usize,
}

/// Rehearse `combo` for `player` from `state`. `None` when `state` is not a
/// position a rehearsal can start from (the player's own main phase, the stack
/// empty, the player to act, loop detection interactive) or the player owns no
/// card of one of the combo's names.
pub fn rehearse(state: &GameState, player: PlayerId, combo: &CatalogCombo) -> Option<Rehearsal> {
    rehearse_within(state, player, combo, Deadline::none())
}

fn rehearse_within(
    state: &GameState,
    player: PlayerId,
    combo: &CatalogCombo,
    deadline: Deadline,
) -> Option<Rehearsal> {
    if !certify::search_may_start(state, player) || combo.cards.is_empty() {
        return None;
    }
    let mut battlefield: Vec<ObjectId> = Vec::new();
    let mut hand: Vec<ObjectId> = Vec::new();
    let mut hand_cost: usize = 0;
    for card in &combo.cards {
        let object = held_copy(state, player, card)?;
        if !matches!(object.zone, Zone::Library | Zone::Hand) {
            // Already out, or somewhere the live search would use it from.
            continue;
        }
        if enters_ahead(object) {
            battlefield.push(object.id);
        } else {
            hand_cost += object.mana_cost.mana_value() as usize;
            if object.mana_cost.has_x() {
                hand_cost += X_SPELL_LANDS;
            }
            hand.push(object.id);
        }
    }
    let lands = (hand_cost + SPARE_LANDS).min(MAX_LANDS);
    let setup = PracticeSetup {
        battlefield: &battlefield,
        hand: &hand,
        lands,
    };
    // A combo that cannot be set out is one this rehearsal cannot vouch for.
    let Some(practice) = practice_position(state, player, &setup) else {
        return Some(Rehearsal::NoWin);
    };
    let (outcome, held_resource) =
        certify::search_line(&practice, player, &combo.cards, deadline, REHEARSAL_WORK);
    if matches!(outcome, SearchOutcome::Found(_)) {
        return Some(Rehearsal::Wins { with: Vec::new() });
    }
    if !held_resource {
        return Some(Rehearsal::NoWin);
    }

    // The combo makes something and has nothing to spend it on. Try again with
    // the deck's X spells in hand.
    let payoffs = x_spells(state, player, &combo.cards);
    if payoffs.is_empty() {
        return Some(Rehearsal::NoWin);
    }
    hand.extend(payoffs.iter().map(|object| object.id));
    let setup = PracticeSetup {
        battlefield: &battlefield,
        hand: &hand,
        lands,
    };
    let Some(practice) = practice_position(state, player, &setup) else {
        return Some(Rehearsal::NoWin);
    };
    let (outcome, _) =
        certify::search_line(&practice, player, &combo.cards, deadline, REHEARSAL_WORK);
    let SearchOutcome::Found(steps) = outcome else {
        return Some(Rehearsal::NoWin);
    };
    let mut with: Vec<String> = Vec::new();
    for id in steps.iter().filter_map(certify::LineStep::cast) {
        if let Some(payoff) = payoffs.iter().find(|object| object.id == id) {
            if !with.contains(&payoff.name) {
                with.push(payoff.name.clone());
            }
        }
    }
    Some(Rehearsal::Wins { with })
}

/// `player`'s copy of the card named `name`: one already on the battlefield if
/// there is one, then one in hand, then anywhere else. The lowest id of equals,
/// so the answer does not depend on map order.
fn held_copy<'a>(state: &'a GameState, player: PlayerId, name: &str) -> Option<&'a GameObject> {
    let rank = |object: &GameObject| match object.zone {
        Zone::Battlefield => 0,
        Zone::Hand => 1,
        Zone::Library => 3,
        _ => 2,
    };
    state
        .objects
        .values()
        .filter(|object| {
            object.name.eq_ignore_ascii_case(name)
                && object.zone != Zone::Stack
                && match object.zone {
                    Zone::Battlefield => object.controller == player,
                    _ => object.owner == player,
                }
        })
        .min_by_key(|object| (rank(object), object.id))
}

/// Whether a practice position puts this card onto the battlefield rather than
/// in hand. A card that is cast for X has to be cast; an Aura has to be cast at
/// something; an instant or sorcery is only ever cast.
fn enters_ahead(object: &GameObject) -> bool {
    let types = &object.card_types;
    !types.core_types.contains(&CoreType::Instant)
        && !types.core_types.contains(&CoreType::Sorcery)
        && !types.subtypes.iter().any(|subtype| subtype == "Aura")
        && !object.mana_cost.has_x()
}

/// The X spells in `player`'s library and hand that `named` does not already
/// list, one of each name, in name order.
fn x_spells<'a>(state: &'a GameState, player: PlayerId, named: &[String]) -> Vec<&'a GameObject> {
    let mut spells: Vec<&GameObject> = state
        .objects
        .values()
        .filter(|object| {
            object.owner == player
                && matches!(object.zone, Zone::Library | Zone::Hand)
                && object.mana_cost.has_x()
                && !named
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(&object.name))
        })
        .collect();
    spells.sort_unstable_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.id.cmp(&right.id))
    });
    spells.dedup_by(|later, earlier| later.name == earlier.name);
    spells.truncate(MAX_PAYOFFS);
    spells
}

/// The combo this seat would rehearse now, if one is due: none this turn yet,
/// fewer than [`MAX_REHEARSALS`] this game, and a combo not yet rehearsed whose
/// cards are not all out (the live search answers for one that is).
fn next_to_rehearse<'a>(
    state: &GameState,
    player: PlayerId,
    session: &AiSession,
    combos: &'a [CatalogCombo],
) -> Option<&'a CatalogCombo> {
    let plans = session.combo_plans.read().ok()?;
    let plan = plans.get(&player);
    if plan
        .is_some_and(|plan| plan.last_turn == Some(state.turn_number) || plan.run >= MAX_REHEARSALS)
    {
        return None;
    }
    combos.iter().find(|combo| {
        !plan.is_some_and(|plan| plan.known.contains_key(&combo.id))
            && !combo
                .cards
                .iter()
                .all(|card| certify::usable(state, player, card))
    })
}

/// Rehearse the next combo of `player`'s that is due, and keep what it showed.
/// The caller has checked that a search may start here.
pub(crate) fn rehearse_due(
    state: &GameState,
    player: PlayerId,
    config: &AiConfig,
    session: &AiSession,
    combos: &[CatalogCombo],
) {
    let Some(combo) = next_to_rehearse(state, player, session, combos) else {
        return;
    };
    let deadline = if config.execution_mode.is_measurement() {
        Deadline::none()
    } else {
        match config.search.time_budget_ms {
            Some(0) => return,
            Some(budget) => Deadline::after(budget.min(REHEARSAL_BUDGET_MS)),
            None => Deadline::after(REHEARSAL_BUDGET_MS),
        }
    };
    let outcome = rehearse_within(state, player, combo, deadline).unwrap_or(Rehearsal::NoWin);
    tracing::debug!(?player, combo = %combo.id, ?outcome, "combo rehearsed");
    if let Ok(mut plans) = session.combo_plans.write() {
        let plan = plans.entry(player).or_default();
        plan.known.insert(combo.id.clone(), outcome);
        plan.last_turn = Some(state.turn_number);
        plan.run += 1;
    }
}

/// Whether `player`'s next decision at `state` would rehearse a combo or search
/// for a combo line. Both need the game's loop detection to be interactive, so
/// a host that shows the AI a copy of the game with detection off (it is
/// slower) asks this first and hands over the game itself when it is true.
pub fn combo_work_due(state: &GameState, session: &AiSession, player: PlayerId) -> bool {
    let Some(combos) = session.combos.get(&player) else {
        return false;
    };
    !combos.is_empty()
        && certify::at_open_main(state, player)
        && (next_to_rehearse(state, player, session, combos).is_some()
            || certify::search_pending(state, player, session, combos))
}

/// How much `player` wants each card it does not have yet, by lowercased name:
/// 1 for the last card a combo is missing, a half for one of two, and so on.
/// Only combos a rehearsal won with count, and only while every missing card is
/// still in the library to be found.
pub(crate) fn wanted_cards(
    state: &GameState,
    session: &AiSession,
    player: PlayerId,
) -> HashMap<String, f64> {
    let mut wanted: HashMap<String, f64> = HashMap::new();
    let Some(combos) = session.combos.get(&player) else {
        return wanted;
    };
    let Ok(plans) = session.combo_plans.read() else {
        return wanted;
    };
    let Some(plan) = plans.get(&player) else {
        return wanted;
    };
    let in_library = |name: &str| {
        state.objects.values().any(|object| {
            object.owner == player
                && object.zone == Zone::Library
                && object.name.eq_ignore_ascii_case(name)
        })
    };
    for combo in combos.iter() {
        let Some(Rehearsal::Wins { with }) = plan.known.get(&combo.id) else {
            continue;
        };
        let missing: Vec<&String> = combo
            .cards
            .iter()
            .chain(with)
            .filter(|card| !certify::usable(state, player, card))
            .collect();
        if missing.is_empty() || !missing.iter().all(|card| in_library(card)) {
            continue;
        }
        let weight = 1.0 / missing.len() as f64;
        for card in missing {
            let entry = wanted.entry(card.to_lowercase()).or_insert(0.0);
            *entry = entry.max(weight);
        }
    }
    wanted
}
