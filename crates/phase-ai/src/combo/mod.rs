//! Combo-recognition layer for cEDH difficulty.
//!
//! - `line.rs` — pure types (`ComboLine`, `ComboPiece`, `ComboReachability`, ...)
//! - `detection.rs` — `ComboDetector` trait + structural impl over `GameState`
//! - `registry.rs` — hand-authored `ComboRegistry`
//!
//! `ComboLinePolicy` (in `policies/combo_line.rs`) wires this layer into the
//! existing planner via `TacticalPolicy::activation()` keyed on
//! `DeckFeatures::is_cedh`.
//!
//! Beside it, for any deck:
//!
//! - `catalog.rs` — a host-supplied `ComboCatalog` and its join against a deck
//! - `certify.rs` — the search that finds a winning line for a catalogued combo
//!   on copies of the game, and the playing-out of a line the engine confirmed

pub mod catalog;
pub(crate) mod certify;
pub mod detection;
pub mod line;
pub mod registry;

pub use catalog::{CatalogCombo, ComboCatalog};
pub use detection::{ComboDetector, StructuralComboDetector};
pub use line::{
    CardPredicate, ComboLine, ComboLineId, ComboPiece, ComboReachability, ComboStep, WinKind,
};
pub use registry::ComboRegistry;
