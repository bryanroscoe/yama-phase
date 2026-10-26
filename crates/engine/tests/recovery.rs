//! Focused runtime regressions for the recovered Yama engine batch.
//!
//! Reuse the same test modules as the full integration target, without linking
//! its two thousand unrelated modules for each iteration of this batch.

#[path = "integration/addendum_those_creatures_relay.rs"]
mod addendum_those_creatures_relay;
#[path = "integration/bestow_normal_cast_static_subject.rs"]
mod bestow_normal_cast_static_subject;
#[path = "integration/chosen_player_anaphor_quantities.rs"]
mod chosen_player_anaphor_quantities;
#[path = "integration/each_creature_deals_damage_to_itself.rs"]
mod each_creature_deals_damage_to_itself;
#[path = "integration/independent_game_state_conditional_sentences.rs"]
mod independent_game_state_conditional_sentences;
#[path = "integration/player_scope_sacrificed_this_way.rs"]
mod player_scope_sacrificed_this_way;
