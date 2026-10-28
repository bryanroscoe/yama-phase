//! CR 109.5 + CR 608.2c: Shaile, Dean of Radiance's "Put a +1/+1 counter on each
//! creature that entered the battlefield under your control this turn" qualifies
//! who controlled the creature as it entered, which a predicate on its current
//! controller can't represent. The parser keeps it an explicit gap; this pins
//! that the public coverage authority reports the card as unsupported.

use engine::game::coverage::card_face_has_unimplemented_parts;
use engine::parser::parse_oracle_text;
use engine::types::ability::AbilityKind;
use engine::types::card::CardFace;

#[test]
fn shaile_entered_under_your_control_this_turn_is_reported_unsupported() {
    let text = "Flying, vigilance\n{T}: Put a +1/+1 counter on each creature that entered the battlefield under your control this turn.";
    let parsed = parse_oracle_text(
        text,
        "Shaile, Dean of Radiance",
        &[],
        &["Creature".to_string()],
        &[],
    );
    let def = parsed
        .abilities
        .iter()
        .find(|def| def.kind == AbilityKind::Activated)
        .expect("Shaile's counter activation");
    let face = CardFace {
        name: "Shaile, Dean of Radiance".to_string(),
        oracle_text: Some(text.to_string()),
        // Only the affected activation, so the keyword line can't supply the gap.
        abilities: vec![def.clone()],
        ..Default::default()
    };
    assert!(card_face_has_unimplemented_parts(&face), "{face:#?}");
}
