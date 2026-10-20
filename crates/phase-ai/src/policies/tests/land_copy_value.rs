//! Copying a land for its effect: Thespian's Stage becoming a copy of Dark
//! Depths (CR 707.2 + CR 603.8). The copy has no ice counters (it didn't enter
//! the battlefield, so Dark Depths' "enters with ten ice counters" never
//! applied), so the copy's state trigger fires at once: sacrifice it, create
//! Marit Lage, a 20/20 flying indestructible token. The legend rule (CR
//! 704.5j) then asks which Dark Depths to keep; only the copy pays off.
//!
//! The deck lab found phase's AI copying opponents' basic lands with the Stage
//! instead: a copy target scored by creature value alone is 0 for every land.

use engine::game::scenario::{GameScenario, P0, P1};
use engine::types::ability::TargetRef;
use engine::types::actions::GameAction;
use engine::types::counter::CounterType;
use engine::types::game_state::{GameState, WaitingFor};
use engine::types::identifiers::ObjectId;
use engine::types::mana::ManaColor;
use engine::types::phase::Phase;
use rand::rngs::SmallRng;
use rand::SeedableRng;

use crate::config::{create_config, AiDifficulty, Platform};
use crate::search::choose_action;

const STAGE: &str =
    "{T}: Add {C}.\n{2}, {T}: This land becomes a copy of target land, except it has this ability.";
const DEPTHS: &str = "Dark Depths enters with ten ice counters on it.\n{3}: Remove an ice counter from Dark Depths.\nWhen Dark Depths has no ice counters on it, sacrifice it. If you do, create Marit Lage, a legendary 20/20 black Avatar creature token with flying and indestructible.";

struct StageBoard {
    state: GameState,
    stage: ObjectId,
    depths: ObjectId,
    opponent_land: ObjectId,
}

/// P0: Thespian's Stage, Dark Depths (ten ice counters), and two Wastes-like
/// lands to pay {2}. P1: a basic land. P0 has activated the Stage and is
/// choosing its target.
fn stage_targeting() -> StageBoard {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let stage = scenario
        .add_land_from_oracle(P0, "Thespian's Stage", STAGE)
        .id();
    let depths = scenario
        .add_land_from_oracle(P0, "Dark Depths", DEPTHS)
        .as_legendary()
        .as_snow()
        .id();
    scenario.with_counter(depths, CounterType::Generic("ice".to_string()), 10);
    scenario.add_basic_land(P0, ManaColor::Green);
    scenario.add_basic_land(P0, ManaColor::Blue);
    let opponent_land = scenario.add_basic_land(P1, ManaColor::Black);
    let mut runner = scenario.build();
    let stage_copy_ability = runner.state().objects[&stage]
        .abilities
        .iter()
        .position(|ability| {
            matches!(
                &*ability.effect,
                engine::types::ability::Effect::BecomeCopy { .. }
            )
        })
        .expect("the Stage's copy ability parses");
    runner
        .act(GameAction::ActivateAbility {
            source_id: stage,
            ability_index: stage_copy_ability,
        })
        .expect("the Stage can be activated");
    assert!(
        matches!(
            runner.state().waiting_for,
            WaitingFor::TargetSelection { .. }
        ),
        "the Stage asks for a target land, got {:?}",
        runner.state().waiting_for
    );
    StageBoard {
        state: runner.state().clone(),
        stage,
        depths,
        opponent_land,
    }
}

/// Answer every prompt for P0 with the AI (Easy, seeded) until the stack is
/// empty and P0 has priority again, or 200 steps pass.
fn play_out(mut state: GameState, seed: u64) -> GameState {
    let config = create_config(AiDifficulty::Easy, Platform::Native);
    let mut rng = SmallRng::seed_from_u64(seed);
    for _ in 0..200 {
        if matches!(state.waiting_for, WaitingFor::Priority { .. }) && state.stack.is_empty() {
            break;
        }
        let actor = state.waiting_for.acting_player().unwrap_or(P0);
        let action = if actor == P0 {
            choose_action(&state, P0, &config, &mut rng).expect("AI answers")
        } else {
            GameAction::PassPriority
        };
        engine::game::apply_as_current(&mut state, action).expect("legal action");
    }
    state
}

fn has_marit_lage(state: &GameState) -> bool {
    state
        .battlefield
        .iter()
        .filter_map(|id| state.objects.get(id))
        .any(|object| object.name == "Marit Lage" && object.controller == P0)
}

#[test]
fn engine_makes_marit_lage_when_the_stage_copy_is_kept() {
    // Rules check, no AI: copy Dark Depths, keep the copy at the legend rule.
    let board = stage_targeting();
    let mut state = board.state;
    engine::game::apply_as_current(
        &mut state,
        GameAction::ChooseTarget {
            target: Some(TargetRef::Object(board.depths)),
        },
    )
    .expect("Dark Depths is a legal target");
    for _ in 0..100 {
        match &state.waiting_for {
            WaitingFor::ChooseLegend { .. } => {
                engine::game::apply_as_current(
                    &mut state,
                    GameAction::ChooseLegend { keep: board.stage },
                )
                .expect("keep the Stage copy");
            }
            WaitingFor::Priority { .. } if state.stack.is_empty() && has_marit_lage(&state) => {
                break
            }
            _ => {
                let auto = engine::ai_support::legal_actions(&state);
                let action = auto
                    .iter()
                    .find(|action| matches!(action, GameAction::PassPriority))
                    .or_else(|| auto.first())
                    .cloned()
                    .expect("some legal action");
                engine::game::apply_as_current(&mut state, action).expect("legal");
            }
        }
    }
    assert!(
        has_marit_lage(&state),
        "keeping the copy makes Marit Lage; battlefield: {:?}",
        state
            .battlefield
            .iter()
            .filter_map(|id| state.objects.get(id).map(|o| o.name.clone()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn easy_ai_copies_dark_depths_and_makes_marit_lage() {
    let mut depths_targets = 0;
    let mut opponent_targets = 0;
    let mut lages = 0;
    for seed in 0..20u64 {
        let board = stage_targeting();
        let config = create_config(AiDifficulty::Easy, Platform::Native);
        let mut rng = SmallRng::seed_from_u64(seed);
        let action = choose_action(&board.state, P0, &config, &mut rng).expect("AI answers");
        match &action {
            GameAction::ChooseTarget {
                target: Some(TargetRef::Object(id)),
            } if *id == board.depths => depths_targets += 1,
            GameAction::ChooseTarget {
                target: Some(TargetRef::Object(id)),
            } if *id == board.opponent_land => opponent_targets += 1,
            _ => {}
        }
        let mut state = board.state;
        engine::game::apply_as_current(&mut state, action).expect("legal target");
        let state = play_out(state, seed);
        if has_marit_lage(&state) {
            lages += 1;
        }
    }
    eprintln!(
        "Stage targets (20 seeds): Dark Depths {depths_targets}, opponent's land {opponent_targets}; Marit Lage {lages}"
    );
    assert!(
        depths_targets >= 16,
        "the Stage should copy Dark Depths: Dark Depths {depths_targets}, opponent's land {opponent_targets} of 20"
    );
    assert!(lages >= 14, "Marit Lage in {lages} of 20 games");
}
