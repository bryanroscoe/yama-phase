//! CR 120.1 + CR 608.2c: "the greatest amount of damage dealt by a source to a
//! permanent or player this turn" has no representation, so the instruction whose
//! amount it is stays a gap. This pins that the public coverage authority reports
//! such cards as unsupported.

use engine::game::coverage::card_face_has_unimplemented_parts;
use engine::parser::parse_oracle_text;
use engine::types::card::CardFace;

#[test]
fn unsupported_source_recipient_damage_quantity_is_reported_unsupported() {
    let text = "Draw X cards, where X is the greatest amount of damage dealt by a source to a permanent or player this turn.";
    let parsed = parse_oracle_text(
        text,
        "Unsupported damage quantity",
        &[],
        &["Sorcery".to_string()],
        &[],
    );
    assert_eq!(parsed.abilities.len(), 1, "{parsed:#?}");
    let face = CardFace {
        name: "Unsupported damage quantity".to_string(),
        oracle_text: Some(text.to_string()),
        abilities: vec![parsed.abilities[0].clone()],
        ..Default::default()
    };
    assert!(card_face_has_unimplemented_parts(&face), "{face:#?}");
}

#[test]
fn impact_resonance_is_reported_unsupported() {
    let text = "Impact Resonance deals X damage divided as you choose among any number of target creatures, where X is the greatest amount of damage dealt by a source to a permanent or player this turn.";
    let parsed = parse_oracle_text(text, "Impact Resonance", &[], &["Instant".to_string()], &[]);
    let face = CardFace {
        name: "Impact Resonance".to_string(),
        oracle_text: Some(text.to_string()),
        abilities: parsed.abilities,
        ..Default::default()
    };
    assert!(card_face_has_unimplemented_parts(&face), "{face:#?}");
}
