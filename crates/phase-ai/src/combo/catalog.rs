//! A combo catalog the host supplies, and its join against a deck.
//!
//! The catalog is data: which cards are known to combine. phase-ai ships none.
//! A host that has one (a Commander Spellbook export, a hand-kept list) builds a
//! [`ComboCatalog`] and gives it to [`crate::session::AiSession::with_combo_catalog`].
//! The session keeps, per seat, the entries whose cards are all in that seat's
//! deck. Nothing here says what a combo does or how to play it: that is found by
//! [`super::certify`], which plays the named cards on a copy of the game and keeps
//! only a line the engine confirms.

use std::collections::HashSet;

use engine::game::DeckEntry;
use serde::{Deserialize, Serialize};

/// One known combination of cards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogCombo {
    /// The host's identifier for this entry, kept for its reports.
    pub id: String,
    /// The names of the cards that combine, with any payoff the combo needs.
    pub cards: Vec<String>,
}

/// The combos a host knows about.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComboCatalog {
    pub combos: Vec<CatalogCombo>,
}

impl ComboCatalog {
    /// The entries whose cards are all in `deck`. Names compare without regard to
    /// case; an entry with no cards matches nothing.
    pub fn combos_in_deck(&self, deck: &[DeckEntry]) -> Vec<CatalogCombo> {
        let names: HashSet<String> = deck
            .iter()
            .map(|entry| entry.card.name.to_lowercase())
            .collect();
        self.combos
            .iter()
            .filter(|combo| {
                !combo.cards.is_empty()
                    && combo
                        .cards
                        .iter()
                        .all(|card| names.contains(&card.to_lowercase()))
            })
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::types::card::CardFace;

    fn entry(name: &str) -> DeckEntry {
        DeckEntry {
            card: CardFace {
                name: name.to_string(),
                ..CardFace::default()
            },
            count: 1,
        }
    }

    fn combo(id: &str, cards: &[&str]) -> CatalogCombo {
        CatalogCombo {
            id: id.to_string(),
            cards: cards.iter().map(|card| card.to_string()).collect(),
        }
    }

    #[test]
    fn a_combo_joins_only_a_deck_that_holds_every_card() {
        let catalog = ComboCatalog {
            combos: vec![
                combo("both", &["Alpha", "Beta"]),
                combo("one-missing", &["Alpha", "Gamma"]),
                combo("empty", &[]),
            ],
        };
        let deck = [entry("alpha"), entry("Beta"), entry("Delta")];
        let joined = catalog.combos_in_deck(&deck);
        assert_eq!(joined, vec![combo("both", &["Alpha", "Beta"])]);
    }

    #[test]
    fn a_catalog_reads_from_json() {
        let catalog: ComboCatalog =
            serde_json::from_str(r#"{"combos":[{"id":"7","cards":["Alpha","Beta"]}]}"#)
                .expect("the catalog parses");
        assert_eq!(catalog.combos, vec![combo("7", &["Alpha", "Beta"])]);
    }
}
