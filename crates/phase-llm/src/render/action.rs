//! Human-readable rendering of an engine [`GameAction`].
//!
//! Written as a generic walk over the action's own serialized payload rather
//! than a match over its ~100 variants. That is deliberate and is the only
//! approach that satisfies "build for the class, not the card": a new
//! `GameAction` variant is described correctly the day it is added, with no
//! second registration point to forget. The engine's `#[serde(tag = "type",
//! content = "data")]` representation and its `strum::IntoStaticStr` variant
//! name are the two facts this relies on, and both are structural.

use std::collections::BTreeMap;

use engine::types::actions::GameAction;
use engine::types::game_state::GameState;
use engine::types::identifiers::ObjectId;
use engine::types::player::PlayerId;
use serde_json::Value;

use super::text::{one_line, seat_label};

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
    /// `Some` names that seat "You" and every other seat by its id, as the
    /// board does ([`seat_label`]); `None` is a seat-neutral `Player N`.
    viewer: Option<PlayerId>,
}

impl Scope<'_> {
    fn player(&self, id: u64) -> String {
        match (self.viewer, u8::try_from(id)) {
            (Some(viewer), Ok(seat)) => seat_label(PlayerId(seat), viewer),
            _ => format!("Player {id}"),
        }
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
    let verb = humanize_identifier(<&'static str>::from(action));
    let Ok(value) = serde_json::to_value(action) else {
        return verb;
    };
    // A unit variant (`PassPriority`) has no payload; the verb is the whole
    // description.
    describe_payload(scope, verb, value.get("data"))
}

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
        return map_scalars(value, |number| {
            object_name(scope.state, ObjectId(number))
                .unwrap_or_else(|| format!("object #{number}"))
        });
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

    #[test]
    fn player_fields_render_as_players_and_object_fields_do_not() {
        assert!(names_players("controller"));
        assert!(!names_players("source_id"));
        assert!(names_objects("targets"));
        assert!(names_objects("source_id"));
        assert!(!names_objects("card_id"));
    }
}
