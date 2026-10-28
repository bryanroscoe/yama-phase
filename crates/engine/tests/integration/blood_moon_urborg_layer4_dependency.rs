//! CR 613.8 dependency inside layer 4: Blood Moon and Urborg, Tomb of Yawgmoth.
//!
//! Urborg's "each land is a Swamp" and a "set a land's subtype" effect (Blood
//! Moon's "nonbasic lands are Mountains", Evil Presence's "enchanted land is a
//! Swamp") both apply in layer 4 (CR 613.1d). Setting Urborg's subtype removes
//! the rules-text ability that generates Urborg's effect (CR 305.7), so Urborg's
//! effect depends on the subtype-setting effect (CR 613.8a "existence") and
//! waits for it (CR 613.8b) — no matter which entered first. Adding the Swamp
//! type changes neither which lands are nonbasic nor whom an Aura enchants, so
//! the subtype-setting effects never depend on Urborg's and there is no loop.
//!
//! The control cases use Blanket of Night, whose identical "each land is a
//! Swamp" ability sits on an enchantment that no land effect can remove: it is
//! independent of Blood Moon and the two apply in timestamp order (CR 613.7).
//!
//! Built on verbatim Oracle text (Scryfall). Expected values come from the
//! Urborg ruling of 2021-03-19 ("it won't turn lands into Swamps, no matter in
//! what order those effects started to apply") and CR 305.7 / CR 613.7 / CR 613.8.

use engine::game::effects::attach::attach_to;
use engine::game::layers::evaluate_layers;
use engine::game::scenario::{GameRunner, GameScenario, P0};
use engine::game::zones::move_to_zone;
use engine::types::ability::{Effect, ManaProduction};
use engine::types::card_type::{CoreType, Supertype};
use engine::types::identifiers::ObjectId;
use engine::types::mana::ManaColor;
use engine::types::phase::Phase;
use engine::types::zones::Zone;

/// Urborg, Tomb of Yawgmoth — Legendary Land — verbatim.
const URBORG: &str = "Each land is a Swamp in addition to its other land types.";
/// Blood Moon {2}{R} Enchantment — verbatim.
const BLOOD_MOON: &str = "Nonbasic lands are Mountains.";
/// Blanket of Night {1}{B}{B} Enchantment — verbatim.
const BLANKET_OF_NIGHT: &str = "Each land is a Swamp in addition to its other land types.";
/// Evil Presence {B} Enchantment — Aura — verbatim.
const EVIL_PRESENCE: &str = "Enchant land\nEnchanted land is a Swamp.";

const URBORG_NAME: &str = "Urborg, Tomb of Yawgmoth";

/// The permanents a test places, in timestamp (placement) order.
#[derive(Clone, Copy)]
enum Permanent {
    Urborg,
    BloodMoon,
    BlanketOfNight,
    Island,
    /// Evil Presence, attached to Urborg at this point (CR 613.7e).
    EvilPresenceOnUrborg,
}

struct Board {
    runner: GameRunner,
    urborg: ObjectId,
    island: ObjectId,
    blood_moon: Option<ObjectId>,
}

fn board(order: &[Permanent]) -> Board {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let placed: Vec<ObjectId> = order
        .iter()
        .map(|permanent| match permanent {
            Permanent::Urborg => scenario
                .add_land_from_oracle(P0, URBORG_NAME, URBORG)
                .as_legendary()
                .id(),
            Permanent::BloodMoon => scenario
                .add_enchantment_from_oracle(P0, "Blood Moon", BLOOD_MOON)
                .id(),
            Permanent::BlanketOfNight => scenario
                .add_enchantment_from_oracle(P0, "Blanket of Night", BLANKET_OF_NIGHT)
                .id(),
            Permanent::Island => scenario.add_basic_land(P0, ManaColor::Blue),
            Permanent::EvilPresenceOnUrborg => scenario
                .add_enchantment_from_oracle(P0, "Evil Presence", EVIL_PRESENCE)
                .with_subtypes(vec!["Aura"])
                .id(),
        })
        .collect();
    let find = |wanted: fn(&Permanent) -> bool| {
        order
            .iter()
            .position(wanted)
            .map(|index| (index, placed[index]))
    };
    let (_, urborg) = find(|p| matches!(p, Permanent::Urborg)).expect("every board has Urborg");
    let (_, island) = find(|p| matches!(p, Permanent::Island)).expect("every board has an Island");
    let blood_moon = find(|p| matches!(p, Permanent::BloodMoon)).map(|(_, id)| id);

    let mut runner = scenario.build();
    if let Some((index, aura)) = find(|p| matches!(p, Permanent::EvilPresenceOnUrborg)) {
        // CR 613.7e: attaching gives the Aura a new timestamp. Re-stamp what the
        // board places after it, so timestamps follow placement order.
        // `attach_to` returns the previous host, so check the new one instead.
        attach_to(runner.state_mut(), aura, urborg);
        assert!(
            runner.state().objects[&urborg].attachments.contains(&aura),
            "Evil Presence must be able to enchant Urborg"
        );
        for &id in &placed[index + 1..] {
            let timestamp = runner.state_mut().next_timestamp();
            runner.state_mut().objects.get_mut(&id).unwrap().timestamp = timestamp;
        }
    }
    relayer(&mut runner);
    Board {
        runner,
        urborg,
        island,
        blood_moon,
    }
}

fn relayer(runner: &mut GameRunner) {
    runner.state_mut().layers_dirty.mark_full();
    evaluate_layers(runner.state_mut());
}

fn sorted_subtypes(runner: &GameRunner, id: ObjectId) -> Vec<String> {
    let mut subtypes = runner.state().objects[&id].card_types.subtypes.clone();
    subtypes.sort();
    subtypes
}

/// The colours of the object's `{T}: Add [one colour]` mana abilities, sorted.
fn mana_colors(runner: &GameRunner, id: ObjectId) -> Vec<ManaColor> {
    let mut colors: Vec<ManaColor> = runner.state().objects[&id]
        .abilities
        .iter()
        .filter_map(|ability| match &*ability.effect {
            Effect::Mana {
                produced: ManaProduction::Fixed { colors, .. },
                ..
            } if colors.len() == 1 => Some(colors[0]),
            _ => None,
        })
        .collect();
    colors.sort_by_key(|color| format!("{color:?}"));
    colors
}

fn assert_types(
    runner: &GameRunner,
    id: ObjectId,
    supertypes: &[Supertype],
    subtypes: &[&str],
    mana: &[ManaColor],
) {
    let obj = &runner.state().objects[&id];
    assert_eq!(
        obj.card_types.core_types,
        vec![CoreType::Land],
        "{}",
        obj.name
    );
    assert_eq!(obj.card_types.supertypes, supertypes, "{}", obj.name);
    let mut expected: Vec<String> = subtypes.iter().map(|s| s.to_string()).collect();
    expected.sort();
    assert_eq!(sorted_subtypes(runner, id), expected, "{}", obj.name);
    let mut expected_mana = mana.to_vec();
    expected_mana.sort_by_key(|color| format!("{color:?}"));
    assert_eq!(
        mana_colors(runner, id),
        expected_mana,
        "{} mana abilities",
        obj.name
    );
}

/// The main board's acceptance tests (spec section 6), shared by the main and
/// reversed-timestamp boards.
fn assert_blood_moon_wins(board: &Board) {
    let runner = &board.runner;
    // CR 305.7: Urborg is a Mountain, keeps its name and legendary supertype,
    // and has only the Mountain's mana ability — no rules-text abilities left.
    assert_types(
        runner,
        board.urborg,
        &[Supertype::Legendary],
        &["Mountain"],
        &[ManaColor::Red],
    );
    let urborg = &runner.state().objects[&board.urborg];
    assert_eq!(urborg.name, URBORG_NAME);
    assert_eq!(urborg.abilities.len(), 1, "only {{T}}: Add {{R}}");
    assert!(
        urborg.static_definitions.is_empty(),
        "CR 305.7: Urborg's static ability is gone"
    );
    // CR 613.8b: Urborg's effect waited for Blood Moon and no longer exists, so
    // the Island is not a Swamp.
    assert_types(
        runner,
        board.island,
        &[Supertype::Basic],
        &["Island"],
        &[ManaColor::Blue],
    );
    let blood_moon = &runner.state().objects[&board.blood_moon.expect("Blood Moon")];
    assert_eq!(
        blood_moon.card_types.core_types,
        vec![CoreType::Enchantment]
    );
    assert!(blood_moon.card_types.subtypes.is_empty());
    // No permanent can produce {B}.
    for id in runner.state().battlefield.iter() {
        assert!(
            !mana_colors(runner, *id).contains(&ManaColor::Black),
            "{} must not tap for black",
            runner.state().objects[id].name
        );
    }
}

/// Main board: Urborg (t1), Blood Moon (t2), Island (t3).
#[test]
fn blood_moon_after_urborg_removes_urborgs_swamp_effect() {
    let board = board(&[Permanent::Urborg, Permanent::BloodMoon, Permanent::Island]);
    assert_blood_moon_wins(&board);
}

/// (a) Reversed timestamps: Blood Moon (t1), Urborg (t2), Island (t3).
#[test]
fn blood_moon_before_urborg_gives_the_same_result() {
    let board = board(&[Permanent::BloodMoon, Permanent::Urborg, Permanent::Island]);
    assert_blood_moon_wins(&board);
}

/// (b1) Urborg, Blood Moon, Island, Blanket of Night: Blanket is independent
/// of Blood Moon and later, so every land ends up a Swamp (CR 613.7, 305.7).
#[test]
fn later_blanket_of_night_makes_every_land_a_swamp_under_blood_moon() {
    let board = board(&[
        Permanent::Urborg,
        Permanent::BloodMoon,
        Permanent::Island,
        Permanent::BlanketOfNight,
    ]);
    assert_types(
        &board.runner,
        board.urborg,
        &[Supertype::Legendary],
        &["Mountain", "Swamp"],
        &[ManaColor::Red, ManaColor::Black],
    );
    assert_types(
        &board.runner,
        board.island,
        &[Supertype::Basic],
        &["Island", "Swamp"],
        &[ManaColor::Blue, ManaColor::Black],
    );
}

/// (b2) Urborg, Blanket of Night, Blood Moon, Island: Blanket applies first and
/// Blood Moon then overwrites Urborg's land types (CR 613.7, 305.7).
#[test]
fn earlier_blanket_of_night_is_overwritten_on_urborg_by_blood_moon() {
    let board = board(&[
        Permanent::Urborg,
        Permanent::BlanketOfNight,
        Permanent::BloodMoon,
        Permanent::Island,
    ]);
    assert_types(
        &board.runner,
        board.urborg,
        &[Supertype::Legendary],
        &["Mountain"],
        &[ManaColor::Red],
    );
    assert_types(
        &board.runner,
        board.island,
        &[Supertype::Basic],
        &["Island", "Swamp"],
        &[ManaColor::Blue, ManaColor::Black],
    );
}

/// (c1) Urborg, Island: Urborg makes each land a Swamp, itself included.
#[test]
fn urborg_alone_makes_every_land_a_swamp() {
    let board = board(&[Permanent::Urborg, Permanent::Island]);
    assert_types(
        &board.runner,
        board.urborg,
        &[Supertype::Legendary],
        &["Swamp"],
        &[ManaColor::Black],
    );
    assert_types(
        &board.runner,
        board.island,
        &[Supertype::Basic],
        &["Island", "Swamp"],
        &[ManaColor::Blue, ManaColor::Black],
    );
}

/// (c2) Urborg, Island, Evil Presence on Urborg: any effect that sets Urborg's
/// subtype removes its ability (CR 305.7, 613.8a/b), not only Blood Moon.
#[test]
fn evil_presence_on_urborg_removes_urborgs_swamp_effect() {
    let board = board(&[
        Permanent::Urborg,
        Permanent::Island,
        Permanent::EvilPresenceOnUrborg,
    ]);
    assert_types(
        &board.runner,
        board.urborg,
        &[Supertype::Legendary],
        &["Swamp"],
        &[ManaColor::Black],
    );
    assert_types(
        &board.runner,
        board.island,
        &[Supertype::Basic],
        &["Island"],
        &[ManaColor::Blue],
    );
}

/// (c3) Urborg, Blood Moon, Evil Presence on Urborg, Island: the two
/// subtype-setting effects are independent, so the later Aura wins (CR 613.7,
/// 613.7e); Urborg's effect depends on both and never applies.
#[test]
fn later_evil_presence_beats_blood_moon_on_urborg() {
    let board = board(&[
        Permanent::Urborg,
        Permanent::BloodMoon,
        Permanent::EvilPresenceOnUrborg,
        Permanent::Island,
    ]);
    assert_types(
        &board.runner,
        board.urborg,
        &[Supertype::Legendary],
        &["Swamp"],
        &[ManaColor::Black],
    );
    assert_types(
        &board.runner,
        board.island,
        &[Supertype::Basic],
        &["Island"],
        &[ManaColor::Blue],
    );
}

/// (c3) With Evil Presence attached before Blood Moon entered, Blood Moon is
/// later and wins.
#[test]
fn later_blood_moon_beats_evil_presence_on_urborg() {
    let board = board(&[
        Permanent::Urborg,
        Permanent::Island,
        Permanent::EvilPresenceOnUrborg,
        Permanent::BloodMoon,
    ]);
    assert_types(
        &board.runner,
        board.urborg,
        &[Supertype::Legendary],
        &["Mountain"],
        &[ManaColor::Red],
    );
    assert_types(
        &board.runner,
        board.island,
        &[Supertype::Basic],
        &["Island"],
        &[ManaColor::Blue],
    );
}

/// (c4) Static effects aren't locked in (CR 611.3a): once Blood Moon leaves,
/// Urborg has its ability again.
#[test]
fn urborg_works_again_after_blood_moon_leaves() {
    let mut board = board(&[Permanent::Urborg, Permanent::BloodMoon, Permanent::Island]);
    assert_blood_moon_wins(&board);

    let blood_moon = board.blood_moon.take().expect("Blood Moon");
    move_to_zone(
        board.runner.state_mut(),
        blood_moon,
        Zone::Graveyard,
        &mut Vec::new(),
    );
    relayer(&mut board.runner);

    assert_types(
        &board.runner,
        board.urborg,
        &[Supertype::Legendary],
        &["Swamp"],
        &[ManaColor::Black],
    );
    assert_types(
        &board.runner,
        board.island,
        &[Supertype::Basic],
        &["Island", "Swamp"],
        &[ManaColor::Blue, ManaColor::Black],
    );
}
