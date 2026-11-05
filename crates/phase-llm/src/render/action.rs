//! Human-readable rendering of an engine [`GameAction`].
//!
//! Written as a generic walk over the action's own serialized payload rather
//! than a match over its ~100 variants. That is deliberate and is the only
//! approach that satisfies "build for the class, not the card": a new
//! `GameAction` variant is described correctly the day it is added, with no
//! second registration point to forget. The engine's `#[serde(tag = "type",
//! content = "data")]` representation and its `strum::IntoStaticStr` variant
//! name are the two facts this relies on, and both are structural.
//!
//! The walk is the default, not the whole story: an action whose payload is
//! only an index or flag into other state (an ability index, a pay-or-not
//! flag, a trigger permutation) is described from that state instead (see
//! `describe_by_meaning`), and falls back to the walk when it cannot be.

use std::collections::BTreeMap;

use engine::types::actions::GameAction;
use engine::types::game_state::GameState;
use engine::types::identifiers::ObjectId;
use engine::types::player::PlayerId;
use engine::types::zones::Zone;
use serde_json::Value;

use super::text::{clamp_text, one_line, seat_label};

/// Payload fields whose numeric values name a player rather than an object.
/// An explicit list, not a heuristic: guessing wrong here would print
/// "Player 7" for an object id.
///
/// Kept complete against the engine's own types: a test reads every
/// `PlayerId`-typed field of `GameAction` and `WaitingFor` from the engine
/// source and fails when one is missing here.
const PLAYER_FIELDS: &[&str] = &[
    "player",
    "players",
    "player_id",
    "controller",
    "owner",
    "opponent",
    "recipient",
    "chooser",
    "representative",
    "defender",
    "attacking_player",
    "target_player",
    // CR 310.8a: the player chosen to protect a battle.
    "protector",
    "caster",
    "defending_player",
    "enters_under_player",
    "library_owner",
    "persist_player",
    "predicted_winner",
    "proposer",
    "pw_controller",
    "remaining_players",
    "scoped_players",
    "source_controller",
    "trigger_controller",
    "winner",
];

/// Fields carrying object ids that do not follow the `_id` suffix convention.
const OBJECT_FIELDS: &[&str] = &["targets", "attackers", "blockers", "cards", "permanents"];

/// What a description is rendered against: the state object ids resolve in,
/// and the seat it is written for.
#[derive(Clone, Copy)]
struct Scope<'a> {
    state: &'a GameState,
    /// `Some` is a prompt for that seat: it reads as "You", every other seat
    /// by its id as the board names it ([`seat_label`]), and every object by
    /// name AND number ([`object_label`]). `None` is a seat-neutral
    /// description: `Player N`, and objects by name alone.
    viewer: Option<PlayerId>,
}

impl Scope<'_> {
    fn player(&self, id: u64) -> String {
        match (self.viewer, u8::try_from(id)) {
            (Some(viewer), Ok(seat)) => seat_label(PlayerId(seat), viewer),
            _ => format!("Player {id}"),
        }
    }

    fn object(&self, id: u64) -> String {
        if self.viewer.is_some() {
            return object_label(self.state, ObjectId(id));
        }
        object_name(self.state, ObjectId(id)).unwrap_or_else(|| format!("object #{id}"))
    }
}

/// `Aethersphere Harvester (Object 192)`: an object as a prompt names it.
///
/// Two options on two permanents that share a name — three Aethersphere
/// Harvesters, a token and the card it copies — must not read the same, and
/// the name alone cannot tell them apart. The object number is the engine's
/// identity for the object, so it is the one handle that both separates such
/// options and ties each one to whatever else names that object.
pub fn object_label(state: &GameState, id: ObjectId) -> String {
    match object_name(state, id) {
        Some(name) => format!("{name} (Object {})", id.0),
        None => format!("Object {}", id.0),
    }
}

/// A one-line description of what `action` does, with object ids resolved to
/// card names against the live state. Seat-neutral: every player is
/// `Player N`. A prompt uses [`describe_action_for`].
pub fn describe_action(state: &GameState, action: &GameAction) -> String {
    describe_action_in(
        Scope {
            state,
            viewer: None,
        },
        action,
    )
}

/// [`describe_action`] as `viewer` reads it: that seat is "You" and every
/// other seat is named as the board names it.
pub fn describe_action_for(state: &GameState, action: &GameAction, viewer: PlayerId) -> String {
    describe_action_in(
        Scope {
            state,
            viewer: Some(viewer),
        },
        action,
    )
}

fn describe_action_in(scope: Scope<'_>, action: &GameAction) -> String {
    if let Some(described) = describe_by_meaning(scope, action) {
        return described;
    }
    let verb = humanize_identifier(<&'static str>::from(action));
    let Ok(value) = serde_json::to_value(action) else {
        return verb;
    };
    // A unit variant (`PassPriority`) has no payload; the verb is the whole
    // description.
    describe_payload(scope, verb, value.get("data"))
}

/// What an option DOES, for the actions whose payload alone cannot say it.
///
/// The generic walk below describes any action from its fields, which is
/// right for an action whose fields are its meaning (a target, a mode, a
/// number). Some payloads are only indices or flags into state the action
/// refers to — "ability index 2" means nothing without the ability — and a
/// model shown the index has to guess which ability it is. Those actions are
/// described here from the state they point into; every other action, and
/// any of these whose referent is missing, falls through to the generic walk.
fn describe_by_meaning(scope: Scope<'_>, action: &GameAction) -> Option<String> {
    match action {
        // CR 602.1 + CR 602.2: an activated ability is identified by its text
        // ("[Cost]: [Effect.]"), not by its position on the card.
        GameAction::ActivateAbility {
            source_id,
            ability_index,
            ..
        } => {
            let text = scope
                .state
                .objects
                .get(source_id)?
                .abilities
                .get(*ability_index)?
                .description
                .as_deref()
                .map(one_line)
                .filter(|text| !text.is_empty())?;
            Some(format!(
                "Activate: \"{}\"",
                clamp_text(&text, ABILITY_TEXT_BUDGET)
            ))
        }
        // CR 702.66a: the engine offers delve's "exile a card from your
        // graveyard to pay {1}" through the same action as convoke's tap
        // (CR 702.51a). Convoke, improvise and waterbend tap a permanent on
        // the battlefield; the only one of them whose card is in a graveyard
        // is delve, and that card is exiled, not tapped.
        GameAction::TapForConvoke { object_id, .. }
            if scope
                .state
                .objects
                .get(object_id)
                .is_some_and(|object| object.zone == Zone::Graveyard) =>
        {
            Some(format!(
                "Exile {} from your graveyard for delve (pays {{1}})",
                scope.object(object_id.0)
            ))
        }
        _ => None,
    }
}

/// Characters of ability text kept in one option.
const ABILITY_TEXT_BUDGET: usize = 320;

/// A one-line description of any engine enum value, read from its serialized
/// form: a bare unit-variant string, an internally tagged object
/// (`{"type": "AtNextPhase", "phase": "End"}`), or an adjacently tagged one
/// (`{"type": .., "data": {..}}`). The same structural walk
/// [`describe_action`] uses, so a new variant of any such enum is described the
/// day it is added. Seat-neutral, like [`describe_action`].
pub fn describe_tagged(state: &GameState, value: &Value) -> String {
    describe_tagged_in(
        Scope {
            state,
            viewer: None,
        },
        value,
    )
}

/// [`describe_tagged`] as `viewer` reads it.
pub fn describe_tagged_for(state: &GameState, value: &Value, viewer: PlayerId) -> String {
    describe_tagged_in(
        Scope {
            state,
            viewer: Some(viewer),
        },
        value,
    )
}

fn describe_tagged_in(scope: Scope<'_>, value: &Value) -> String {
    match value {
        Value::String(variant) => humanize_identifier(variant),
        Value::Object(fields) => {
            let Some(kind) = fields.get("type").and_then(Value::as_str) else {
                return render_value(scope, "", value);
            };
            let verb = humanize_identifier(kind);
            match fields.get("data") {
                Some(data) => describe_payload(scope, verb, Some(data)),
                None => {
                    let rest: serde_json::Map<String, Value> = fields
                        .iter()
                        .filter(|(field, _)| field.as_str() != "type")
                        .map(|(field, item)| (field.clone(), item.clone()))
                        .collect();
                    describe_payload(scope, verb, Some(&Value::Object(rest)))
                }
            }
        }
        other => render_value(scope, "", other),
    }
}

/// `verb` followed by the payload's non-empty fields: `Verb (Field: value, ..)`.
fn describe_payload(scope: Scope<'_>, verb: String, data: Option<&Value>) -> String {
    let Some(data) = data.and_then(Value::as_object) else {
        return verb;
    };

    // `BTreeMap` for a stable field order: the same decision must render
    // identically on every call, or the fingerprint that guards against a stale
    // decision would flap.
    let ordered: BTreeMap<&String, &Value> = data.iter().collect();
    let details: Vec<String> = ordered
        .into_iter()
        .filter(|(_, value)| !is_empty_value(value))
        .map(|(field, value)| {
            format!(
                "{}: {}",
                humanize_identifier(field),
                render_value(scope, field, value)
            )
        })
        .collect();

    // Folded at the exit, not per leaf: a payload string is humanized word by
    // word and can still carry a line break, and every leaf would otherwise need
    // to remember that.
    if details.is_empty() {
        verb
    } else {
        one_line(&format!("{verb} ({})", details.join(", ")))
    }
}

/// The primary object an action operates on, when it has one. Used to lead a
/// candidate line with the card name — the highest-signal token for a model
/// that already knows the card.
pub fn primary_object_name(state: &GameState, action: &GameAction) -> Option<String> {
    // The engine's exhaustive source-object authority, not a list kept here:
    // every action that works on one permanent or card — a mana ability on an
    // artifact or creature as much as a spell cast from hand — leads with that
    // object's name, including variants added after this was written.
    object_name(state, action.source_object()?)
}

/// [`primary_object_name`] with the object's number, as a prompt names it
/// ([`object_label`]).
pub fn primary_object_label(state: &GameState, action: &GameAction) -> Option<String> {
    let id = action.source_object()?;
    state
        .objects
        .contains_key(&id)
        .then(|| object_label(state, id))
}

fn object_name(state: &GameState, id: ObjectId) -> Option<String> {
    state.objects.get(&id).map(|object| one_line(&object.name))
}

/// `snake_case` / `CamelCase` -> `Title Case`, with the repository's existing
/// `id -> ID` convention preserved.
pub fn humanize_identifier(raw: &str) -> String {
    let spaced = split_camel_case(raw);
    spaced
        .split(['_', ' '])
        .filter(|word| !word.is_empty())
        .map(|word| match word.to_ascii_lowercase().as_str() {
            "id" => "ID".to_string(),
            "ids" => "IDs".to_string(),
            _ => {
                let mut chars = word.chars();
                chars.next().map_or_else(String::new, |first| {
                    first.to_uppercase().collect::<String>() + chars.as_str()
                })
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn split_camel_case(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 4);
    for (index, c) in raw.char_indices() {
        if index > 0 && c.is_uppercase() {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

fn is_empty_value(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Array(items) => items.is_empty(),
        Value::Object(fields) => fields.is_empty(),
        Value::String(text) => text.is_empty(),
        _ => false,
    }
}

/// Render one payload field. Object-id fields become card names; player fields
/// become seat labels; everything else falls through to a compact JSON-ish form.
fn render_value(scope: Scope<'_>, field: &str, value: &Value) -> String {
    if names_players(field) {
        return map_scalars(value, |number| scope.player(number));
    }
    if names_objects(field) {
        return map_scalars(value, |number| scope.object(number));
    }
    match value {
        Value::String(text) => humanize_identifier(text),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number.to_string(),
        Value::Null => "none".to_string(),
        Value::Array(items) => items
            .iter()
            .map(|item| render_value(scope, field, item))
            .collect::<Vec<_>>()
            .join(", "),
        Value::Object(fields) => fields
            .iter()
            .map(|(key, item)| match (key.as_str(), item.as_u64()) {
                // `TargetRef::Player` / `AttackTarget::Player`: an externally
                // tagged player reference names a seat like a player field.
                ("Player", Some(number)) => scope.player(number),
                // `TargetRef::Object`: likewise an object reference.
                ("Object", Some(number)) if scope.viewer.is_some() => scope.object(number),
                _ => format!(
                    "{} {}",
                    humanize_identifier(key),
                    render_value(scope, key, item)
                ),
            })
            .collect::<Vec<_>>()
            .join(", "),
    }
}

/// `card_id` is deliberately excluded: it indexes the card database, not the
/// object table, so resolving it as an object would print an unrelated card.
fn names_objects(field: &str) -> bool {
    if field == "card_id" || field == "card_ids" {
        return false;
    }
    OBJECT_FIELDS.contains(&field) || field.ends_with("_id") || field.ends_with("_ids")
}

fn names_players(field: &str) -> bool {
    PLAYER_FIELDS.contains(&field)
}

/// A short description of the decision the engine is waiting on, from the
/// `WaitingFor` variant name and (when the prompt has one) the engine-latched
/// display name of the source that raised it.
pub fn describe_waiting_for(waiting_for: &engine::types::game_state::WaitingFor) -> String {
    let Ok(value) = serde_json::to_value(waiting_for) else {
        return "a decision".to_string();
    };
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .map_or_else(|| "a decision".to_string(), humanize_identifier);
    let source = value
        .get("data")
        .and_then(|data| data.get("source"))
        .and_then(|source| source.get("display_name"))
        .and_then(Value::as_str);
    match source {
        Some(name) => format!("{kind} (from {})", one_line(name)),
        None => kind,
    }
}

/// The decision the engine is waiting on, as a prompt shows it: the kind of
/// decision and, when it belongs to a spell or ability being put on the stack
/// or a trigger choosing targets, which one and what it says.
///
/// `state` is the seat's viewer-filtered state. A bare "Target Selection"
/// leaves the seat to guess what is targeting: the decision before may have
/// been an activation of a different permanent than the one it remembers, and
/// a trigger's targets read like any other's. The engine already carries the
/// answer — the cast or activation in progress (`WaitingFor::pending_cast_ref`,
/// or `GameState::pending_cast` while mana is being paid) and the triggering
/// source and its text — so it is named here.
pub fn describe_decision(state: &GameState) -> String {
    let base = describe_waiting_for(&state.waiting_for);
    match decision_subject(state) {
        Some(subject) => one_line(&format!("{base} for {subject}")),
        None => base,
    }
}

/// `Volatile Fault's activated ability: Destroy target ...` for the spell,
/// ability or trigger a decision belongs to.
fn decision_subject(state: &GameState) -> Option<String> {
    use engine::types::game_state::WaitingFor;

    let pending = match &state.waiting_for {
        WaitingFor::TriggerTargetSelection {
            source_id,
            description,
            ..
        } => {
            // CR 111.7 + CR 400.7: a token (or a copy) whose trigger is
            // choosing targets may already have ceased to exist. Its last
            // known name still says whose trigger it is, and the trigger's
            // latched text says what it does even with no name at all.
            let source = source_id
                .and_then(|id| {
                    object_name(state, id)
                        .or_else(|| state.lki_cache.get(&id).map(|lki| one_line(&lki.name)))
                })
                .filter(|name| !name.is_empty());
            if source.is_none() && description.is_none() {
                return None;
            }
            let subject = match source {
                Some(source) => format!("{source}'s triggered ability"),
                None => "a triggered ability".to_string(),
            };
            return Some(with_text(subject, description.as_deref()));
        }
        // CR 601.2g-h: mana payment does not embed its cast; the cast being
        // paid for is the state's pending cast.
        WaitingFor::ManaPayment { .. } | WaitingFor::ManaSourceSelection { .. } => {
            state.pending_cast.as_deref()?
        }
        other => other.pending_cast_ref()?,
    };
    // CR 400.7: a source that left its zone mid-cast (sacrificed as a cost)
    // still names the ability by its last known name.
    let name = object_name(state, pending.object_id).or_else(|| {
        state
            .lki_cache
            .get(&pending.object_id)
            .map(|lki| one_line(&lki.name))
    })?;
    match pending.activation_ability_index {
        // CR 602.2: an activation in progress belongs to its source's ability.
        Some(index) => {
            let text = pending.ability.description.as_deref().or_else(|| {
                state
                    .objects
                    .get(&pending.object_id)?
                    .abilities
                    .get(index)
                    .and_then(|ability| ability.description.as_deref())
            });
            Some(with_text(format!("{name}'s activated ability"), text))
        }
        None => Some(with_text(name, pending.ability.description.as_deref())),
    }
}

fn with_text(subject: String, text: Option<&str>) -> String {
    match text.map(one_line).filter(|text| !text.is_empty()) {
        Some(text) => format!("{subject}: {}", clamp_text(&text, ABILITY_TEXT_BUDGET)),
        None => subject,
    }
}

/// Apply `render` to every number in `value`, preserving array structure.
fn map_scalars(value: &Value, render: impl Fn(u64) -> String + Copy) -> String {
    match value {
        Value::Number(number) => number.as_u64().map_or_else(|| number.to_string(), render),
        Value::Array(items) => items
            .iter()
            .map(|item| map_scalars(item, render))
            .collect::<Vec<_>>()
            .join(", "),
        Value::Null => "none".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::types::identifiers::CardId;

    fn empty_state() -> GameState {
        GameState::default()
    }

    #[test]
    fn a_unit_variant_renders_as_its_verb_alone() {
        assert_eq!(
            describe_action(&empty_state(), &GameAction::PassPriority),
            "Pass Priority"
        );
    }

    #[test]
    fn a_payload_variant_lists_its_fields_in_a_stable_order() {
        let action = GameAction::PlayLand {
            object_id: ObjectId(7),
            card_id: CardId(3),
        };
        let state = empty_state();
        let rendered = describe_action(&state, &action);
        assert_eq!(rendered, describe_action(&state, &action));
        assert!(rendered.starts_with("Play Land ("), "{rendered}");
        // `card_id` must NOT be resolved through the object table.
        assert!(rendered.contains("Card ID: 3"), "{rendered}");
        assert!(rendered.contains("Object ID: object #7"), "{rendered}");
    }

    /// Every action on one object leads with that object's name — including
    /// mana-source actions, so an artifact a seat can tap for mana reads as
    /// that artifact rather than as an anonymous payload.
    #[test]
    fn the_primary_object_comes_from_the_engines_source_object_authority() {
        use engine::game::create_object;
        use engine::types::mana::ManaType;
        use engine::types::player::PlayerId;
        use engine::types::zones::Zone;

        let mut state = empty_state();
        let rock = create_object(
            &mut state,
            CardId(1),
            PlayerId(0),
            "Mind Stone".to_string(),
            Zone::Battlefield,
        );
        let action = GameAction::TapForConvoke {
            object_id: rock,
            mana_type: ManaType::Colorless,
        };
        assert_eq!(
            primary_object_name(&state, &action).as_deref(),
            Some("Mind Stone")
        );
        assert_eq!(primary_object_name(&state, &GameAction::PassPriority), None);
    }

    #[test]
    fn tagged_values_describe_as_their_variant_and_fields() {
        let state = empty_state();
        let internal = serde_json::json!({ "type": "AtNextPhase", "phase": "End" });
        assert_eq!(
            describe_tagged(&state, &internal),
            "At Next Phase (Phase: End)"
        );
        let adjacent = serde_json::json!({ "type": "WhenLeavesPlay", "data": { "object_id": 9 } });
        assert_eq!(
            describe_tagged(&state, &adjacent),
            "When Leaves Play (Object ID: object #9)"
        );
        assert_eq!(
            describe_tagged(&state, &serde_json::json!("EndOfTurn")),
            "End Of Turn"
        );
    }

    /// Every `PlayerId`-typed field of the engine's `GameAction` and
    /// `WaitingFor`, read from the engine's own source, renders as a seat.
    /// `WaitingFor` reuses three names for players in one variant and for
    /// objects in another (`candidates`, `chosen`, `remaining`); those are
    /// left to the generic walk, which a player field must not shadow.
    #[test]
    fn every_player_typed_field_renders_as_a_seat() {
        const AMBIGUOUS: &[&str] = &["candidates", "chosen", "remaining"];
        let enum_body = |source: &'static str, header: &str| -> &'static str {
            let start = source.find(header).expect("enum header") + header.len();
            let mut depth = 1usize;
            for (offset, c) in source[start..].char_indices() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            return &source[start..start + offset];
                        }
                    }
                    _ => {}
                }
            }
            panic!("unterminated {header}")
        };
        let player_fields = |body: &str| -> Vec<String> {
            body.lines()
                .filter_map(|line| {
                    let line = line.trim().trim_start_matches("pub ");
                    let (name, ty) = line.split_once(':')?;
                    let name = name.trim();
                    let ty = ty.trim().trim_end_matches(',');
                    let wrapped = ty
                        .trim_start_matches("Option<")
                        .trim_start_matches("Vec<")
                        .trim_start_matches("Box<")
                        .trim_start_matches("crate::types::player::");
                    (name.chars().all(|c| c.is_ascii_lowercase() || c == '_')
                        && wrapped.starts_with("PlayerId")
                        && !wrapped["PlayerId".len()..].starts_with(|c: char| c.is_alphanumeric()))
                    .then(|| name.to_string())
                })
                .collect()
        };
        let actions = include_str!("../../../engine/src/types/actions.rs");
        let states = include_str!("../../../engine/src/types/game_state.rs");
        let mut fields = player_fields(enum_body(actions, "pub enum GameAction {"));
        fields.extend(player_fields(enum_body(states, "pub enum WaitingFor {")));
        assert!(fields.contains(&"protector".to_string()), "{fields:?}");
        for field in fields {
            if AMBIGUOUS.contains(&field.as_str()) {
                continue;
            }
            assert!(
                names_players(&field),
                "PlayerId field `{field}` renders as a number"
            );
        }
        for field in AMBIGUOUS {
            assert!(
                !names_players(field),
                "{field} must stay with the generic walk"
            );
        }

        let protector = GameAction::ChooseBattleProtector {
            protector: engine::types::player::PlayerId(2),
        };
        let state = empty_state();
        assert_eq!(
            describe_action_for(&state, &protector, engine::types::player::PlayerId(2)),
            "Choose Battle Protector (Protector: You)"
        );
        assert_eq!(
            describe_action_for(&state, &protector, engine::types::player::PlayerId(0)),
            "Choose Battle Protector (Protector: Player 2)"
        );
    }

    #[test]
    fn identifier_humanization_keeps_the_id_convention() {
        assert_eq!(humanize_identifier("source_id"), "Source ID");
        assert_eq!(humanize_identifier("CastSpell"), "Cast Spell");
        assert_eq!(
            humanize_identifier("card_instance_ids"),
            "Card Instance IDs"
        );
    }

    /// A prompt's options name seats as its board does: the viewer is "You"
    /// and every other seat its 0-based id, in a player field and in a
    /// player target alike.
    #[test]
    fn options_name_seats_as_the_viewer_reads_them() {
        use engine::types::ability::TargetRef;
        use engine::types::player::PlayerId;

        let state = empty_state();
        let target = |player: u8| GameAction::ChooseTarget {
            target: Some(TargetRef::Player(PlayerId(player))),
        };
        assert_eq!(
            describe_action_for(&state, &target(1), PlayerId(1)),
            "Choose Target (Target: You)"
        );
        assert_eq!(
            describe_action_for(&state, &target(2), PlayerId(1)),
            "Choose Target (Target: Player 2)"
        );
        let opponent = GameAction::ChooseEntryController {
            opponent: PlayerId(3),
        };
        assert_eq!(
            describe_action_for(&state, &opponent, PlayerId(3)),
            "Choose Entry Controller (Opponent: You)"
        );
        // The seat-neutral form keeps numbering every seat.
        assert_eq!(
            describe_action(&state, &opponent),
            "Choose Entry Controller (Opponent: Player 3)"
        );
    }

    /// Options on different objects that share a name read differently, by
    /// object number, wherever they name the object.
    #[test]
    fn same_named_objects_are_told_apart_by_number() {
        use engine::game::create_object;
        use engine::types::ability::TargetRef;
        use engine::types::player::PlayerId;
        use engine::types::zones::Zone;

        let mut state = empty_state();
        let harvesters: Vec<ObjectId> = (0..2)
            .map(|index| {
                create_object(
                    &mut state,
                    CardId(index),
                    PlayerId(0),
                    "Aethersphere Harvester".to_string(),
                    Zone::Battlefield,
                )
            })
            .collect();
        let crew = |vehicle_id| GameAction::CrewVehicle {
            vehicle_id,
            creature_ids: Vec::new(),
        };
        let first = describe_action_for(&state, &crew(harvesters[0]), PlayerId(0));
        let second = describe_action_for(&state, &crew(harvesters[1]), PlayerId(0));
        assert_ne!(first, second);
        assert!(
            first.contains(&format!(
                "Aethersphere Harvester (Object {})",
                harvesters[0].0
            )),
            "{first}"
        );
        assert_eq!(
            primary_object_label(&state, &crew(harvesters[1])),
            Some(format!(
                "Aethersphere Harvester (Object {})",
                harvesters[1].0
            ))
        );
        let target = GameAction::ChooseTarget {
            target: Some(TargetRef::Object(harvesters[1])),
        };
        assert_eq!(
            describe_action_for(&state, &target, PlayerId(0)),
            format!(
                "Choose Target (Target: Aethersphere Harvester (Object {}))",
                harvesters[1].0
            )
        );
        // The seat-neutral form is unchanged.
        assert_eq!(
            describe_action(&state, &crew(harvesters[0])),
            "Crew Vehicle (Vehicle ID: Aethersphere Harvester)"
        );
    }

    /// An activated-ability option quotes the ability it activates, so the
    /// ability's index never has to be mapped to the card's text.
    #[test]
    fn an_activate_option_quotes_the_ability_it_activates() {
        use engine::game::create_object;
        use engine::types::ability::{AbilityDefinition, AbilityKind, Effect};
        use engine::types::player::PlayerId;
        use engine::types::zones::Zone;
        use std::sync::Arc;

        let mut state = empty_state();
        let sage = create_object(
            &mut state,
            CardId(1),
            PlayerId(0),
            "Sage of the Maze".to_string(),
            Zone::Battlefield,
        );
        let ability = |text: &str| {
            let mut definition = AbilityDefinition::new(
                AbilityKind::Activated,
                Effect::Unimplemented {
                    name: "test".to_string(),
                    description: None,
                },
            );
            definition.description = Some(text.to_string());
            definition
        };
        state.objects.get_mut(&sage).expect("created").abilities = Arc::new(vec![
            ability("{T}: Add two mana in any combination of colors."),
            ability(
                "{T}: Until end of turn, target land you control becomes an X/X Citizen \
                 creature with haste in addition to its other types, where X is twice the \
                 number of Gates you control. Activate only as a sorcery.",
            ),
            ability("Tap an untapped Gate you control: Untap this creature."),
        ]);
        let activate = |ability_index| GameAction::ActivateAbility {
            source_id: sage,
            ability_index,
            payment_mode: Default::default(),
        };
        assert_eq!(
            describe_action_for(&state, &activate(2), PlayerId(0)),
            "Activate: \"Tap an untapped Gate you control: Untap this creature.\""
        );
        assert_ne!(
            describe_action_for(&state, &activate(1), PlayerId(0)),
            describe_action_for(&state, &activate(2), PlayerId(0))
        );
        // An index with no ability behind it falls back to the generic walk.
        assert!(describe_action_for(&state, &activate(7), PlayerId(0))
            .starts_with("Activate Ability (Ability Index: 7"),);
    }

    /// CR 702.66a: a delve payment exiles a graveyard card. It must not read
    /// as convoke, which taps a permanent.
    #[test]
    fn a_delve_payment_reads_as_delve_and_a_convoke_tap_as_convoke() {
        use engine::game::create_object;
        use engine::types::mana::ManaType;
        use engine::types::player::PlayerId;

        let mut state = empty_state();
        let fuel = create_object(
            &mut state,
            CardId(1),
            PlayerId(0),
            "Urban Evolution".to_string(),
            Zone::Graveyard,
        );
        let elf = create_object(
            &mut state,
            CardId(2),
            PlayerId(0),
            "Llanowar Elves".to_string(),
            Zone::Battlefield,
        );
        let tap = |object_id| GameAction::TapForConvoke {
            object_id,
            mana_type: ManaType::Colorless,
        };
        let delve = describe_action_for(&state, &tap(fuel), PlayerId(0));
        assert_eq!(
            delve,
            format!(
                "Exile Urban Evolution (Object {}) from your graveyard for delve (pays {{1}})",
                fuel.0
            )
        );
        assert!(!delve.contains("Convoke"), "{delve}");
        assert!(describe_action_for(&state, &tap(elf), PlayerId(0)).starts_with("Tap For Convoke"),);
    }

    /// A target choice names what is choosing the targets: the activation in
    /// progress, or the trigger and its text.
    #[test]
    fn a_target_choice_names_the_ability_it_is_for() {
        use engine::game::create_object;
        use engine::types::ability::{AbilityDefinition, AbilityKind, Effect, ResolvedAbility};
        use engine::types::game_state::{PendingCast, WaitingFor};
        use engine::types::mana::ManaCost;
        use engine::types::player::PlayerId;
        use std::sync::Arc;

        let mut state = empty_state();
        assert_eq!(
            describe_decision(&state),
            describe_waiting_for(&state.waiting_for)
        );

        let fault = create_object(
            &mut state,
            CardId(1),
            PlayerId(0),
            "Volatile Fault".to_string(),
            Zone::Battlefield,
        );
        let text =
            "{1}, {T}, Sacrifice this land: Destroy target nonbasic land an opponent controls.";
        let mut definition = AbilityDefinition::new(
            AbilityKind::Activated,
            Effect::Unimplemented {
                name: "test".to_string(),
                description: None,
            },
        );
        definition.description = Some(text.to_string());
        state.objects.get_mut(&fault).expect("created").abilities = Arc::new(vec![definition]);
        let ability = ResolvedAbility::new(
            Effect::Unimplemented {
                name: "test".to_string(),
                description: None,
            },
            Vec::new(),
            fault,
            PlayerId(0),
        );
        let mut pending = PendingCast::new(fault, CardId(1), ability, ManaCost::zero());
        pending.activation_ability_index = Some(0);
        state.waiting_for = WaitingFor::TargetSelection {
            player: PlayerId(0),
            pending_cast: Box::new(pending),
            target_slots: Vec::new(),
            mode_labels: Vec::new(),
            selection: Default::default(),
        };
        assert_eq!(
            describe_decision(&state),
            format!("Target Selection for Volatile Fault's activated ability: {text}")
        );

        let roc = create_object(
            &mut state,
            CardId(2),
            PlayerId(0),
            "Aetherstorm Roc".to_string(),
            Zone::Battlefield,
        );
        state.waiting_for = WaitingFor::TriggerTargetSelection {
            player: PlayerId(0),
            trigger_controller: None,
            trigger_event: None,
            trigger_events: Vec::new(),
            target_slots: Vec::new(),
            mode_labels: Vec::new(),
            target_constraints: Vec::new(),
            selection: Default::default(),
            source_id: Some(roc),
            description: Some(
                "Whenever this creature attacks, tap up to one target creature defending player controls."
                    .to_string(),
            ),
        };
        assert_eq!(
            describe_decision(&state),
            "Trigger Target Selection for Aetherstorm Roc's triggered ability: Whenever this \
             creature attacks, tap up to one target creature defending player controls."
        );
    }

    /// A token's dies trigger chooses its targets after the token has ceased
    /// to exist (CR 111.7): the trigger's own text still names what is
    /// choosing targets.
    #[test]
    fn a_trigger_from_a_vanished_token_still_names_its_text() {
        use engine::game::create_object;
        use engine::types::game_state::WaitingFor;
        use engine::types::player::PlayerId;

        let mut state = empty_state();
        let token = create_object(
            &mut state,
            CardId(1),
            PlayerId(0),
            "Spirit".to_string(),
            Zone::Battlefield,
        );
        state.objects.remove(&token);
        state.lki_cache.remove(&token);
        state.waiting_for = WaitingFor::TriggerTargetSelection {
            player: PlayerId(0),
            trigger_controller: None,
            trigger_event: None,
            trigger_events: Vec::new(),
            target_slots: Vec::new(),
            mode_labels: Vec::new(),
            target_constraints: Vec::new(),
            selection: Default::default(),
            source_id: Some(token),
            description: Some("When this token dies, it deals 1 damage to any target.".to_string()),
        };
        assert_eq!(
            decision_subject(&state).as_deref(),
            Some("a triggered ability: When this token dies, it deals 1 damage to any target.")
        );
    }

    #[test]
    fn player_fields_render_as_players_and_object_fields_do_not() {
        assert!(names_players("controller"));
        assert!(!names_players("source_id"));
        assert!(names_objects("targets"));
        assert!(names_objects("source_id"));
        assert!(!names_objects("card_id"));
    }
}
