//! The show half of lighting state.
//!
//! Looks, lighting decks, and the palettes that travel with a performance. Persisted in
//! `scene.json`, beside channels and decks, because these are artistic choices rather than facts
//! about a room.
//!
//! The counterpart is [`super::config::LightingConfig`] in `stage.json`: fixtures, groups,
//! transports, and position palettes. The split is what lets a show file carry no
//! venue-specific data at all. See /spec/lighting-routing.md § Persistence.

use super::look::Look;
use super::palette::Palette;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Lighting state belonging to the show.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LightingShow {
    /// The library of saved looks.
    #[serde(default)]
    pub looks: Vec<Look>,
    /// Color and beam palettes. Position palettes are venue state and live in `stage.json`.
    #[serde(default)]
    pub palettes: Vec<Palette>,
    /// Global intensity scalar, `lighting/master` in the parameter router.
    #[serde(default = "unity")]
    pub master: f32,
}

fn unity() -> f32 {
    1.0
}

impl Default for LightingShow {
    fn default() -> Self {
        Self {
            looks: Vec::new(),
            palettes: Vec::new(),
            // Unity, matching the serde default. A derived `Default` would give 0.0 and leave a
            // programmatically created show silently dark, which is the kind of divergence that
            // only shows up on stage.
            master: unity(),
        }
    }
}

impl LightingShow {
    /// Find a saved look in the library.
    ///
    /// Deck *content* is not here: a lighting deck belongs to the channel that owns it, so the
    /// mixer is where deck content is addressed. See `Mixer::find_deck_look_mut`.
    #[must_use]
    pub fn look(&self, id: Uuid) -> Option<&Look> {
        self.looks.iter().find(|l| l.id == id)
    }

    #[must_use]
    pub fn look_mut(&mut self, id: Uuid) -> Option<&mut Look> {
        self.looks.iter_mut().find(|l| l.id == id)
    }

    /// True when the show carries no lighting content at all, which is the default.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        // Decks are not counted: they belong to the mixer's channels now, so a show with no
        // saved looks and no palettes carries no lighting content of its own.
        self.looks.is_empty() && self.palettes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_default_show_is_empty_with_unity_master() {
        let show = LightingShow::default();
        assert!(show.is_empty());
        assert!((show.master - 1.0).abs() < f32::EPSILON);
    }

    /// The two default paths must agree. A derived `Default` gives 0.0 for a float, which would
    /// leave a programmatically created show dark while a loaded one was lit.
    #[test]
    fn the_serde_default_and_the_rust_default_agree() {
        let loaded: LightingShow = serde_json::from_str("{}").unwrap();
        assert_eq!(loaded, LightingShow::default());
    }

    #[test]
    fn look_lookup_by_uuid() {
        let mut show = LightingShow::default();
        let look = Look::new("Deep Blue");
        let id = look.id;
        show.looks.push(look);
        assert_eq!(show.look(id).unwrap().name, "Deep Blue");
        show.look_mut(id).unwrap().name = "Deeper Blue".into();
        assert_eq!(show.look(id).unwrap().name, "Deeper Blue");
        assert!(show.look(Uuid::new_v4()).is_none());
    }

    #[test]
    fn a_show_round_trips_through_json() {
        let mut show = LightingShow {
            master: 0.8,
            ..LightingShow::default()
        };
        show.looks.push(Look::new("Deep Blue"));
        let text = serde_json::to_string(&show).unwrap();
        let back: LightingShow = serde_json::from_str(&text).unwrap();
        assert_eq!(show, back);
    }
}
