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
use super::merge::LightingDeck;
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
    /// Lighting decks per channel, parallel to the channel list in the mixer.
    ///
    /// Indexed by channel UUID rather than position, so reordering channels cannot silently
    /// rebind a deck to a different one.
    #[serde(default)]
    pub channel_decks: Vec<ChannelDecks>,
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
            channel_decks: Vec::new(),
            // Unity, matching the serde default. A derived `Default` would give 0.0 and leave a
            // programmatically created show silently dark, which is the kind of divergence that
            // only shows up on stage.
            master: unity(),
        }
    }
}

/// One channel's lighting decks, addressed by the channel's UUID.
///
/// The key is a `String` because Varda channels and decks carry short 8-character identifiers
/// (`generate_short_uuid`), not full RFC 4122 UUIDs. Typing this as `uuid::Uuid` compiles and
/// then silently fails to parse every real channel, which is exactly the kind of bug that only
/// shows up as "lighting never receives channel opacity".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChannelDecks {
    pub channel: String,
    #[serde(default)]
    pub decks: Vec<LightingDeck>,
}

impl LightingShow {
    /// Find a look by id, in deck content first and then the saved library.
    ///
    /// Deck content comes first because that is what the editor addresses: every command from
    /// the deck detail names the content's own id. The library is searched too so a saved look
    /// can still be renamed and inspected.
    #[must_use]
    pub fn look(&self, id: Uuid) -> Option<&Look> {
        self.channel_decks
            .iter()
            .flat_map(|c| &c.decks)
            .map(|d| &d.content)
            .find(|l| l.id == id)
            .or_else(|| self.looks.iter().find(|l| l.id == id))
    }

    #[must_use]
    pub fn look_mut(&mut self, id: Uuid) -> Option<&mut Look> {
        if let Some(found) = self
            .channel_decks
            .iter_mut()
            .flat_map(|c| &mut c.decks)
            .map(|d| &mut d.content)
            .find(|l| l.id == id)
        {
            return Some(found);
        }
        self.looks.iter_mut().find(|l| l.id == id)
    }

    /// Decks for one channel, creating the entry if absent.
    pub fn decks_mut(&mut self, channel: &str) -> &mut Vec<LightingDeck> {
        if let Some(index) = self.channel_decks.iter().position(|c| c.channel == channel) {
            return &mut self.channel_decks[index].decks;
        }
        self.channel_decks.push(ChannelDecks {
            channel: channel.to_owned(),
            decks: Vec::new(),
        });
        let last = self.channel_decks.len() - 1;
        &mut self.channel_decks[last].decks
    }

    #[must_use]
    pub fn decks(&self, channel: &str) -> &[LightingDeck] {
        self.channel_decks
            .iter()
            .find(|c| c.channel == channel)
            .map_or(&[], |c| c.decks.as_slice())
    }

    /// Find a lighting deck anywhere in the show, with the channel it belongs to.
    #[must_use]
    pub fn find_deck(&self, deck: Uuid) -> Option<(&str, &LightingDeck)> {
        self.channel_decks.iter().find_map(|c| {
            c.decks
                .iter()
                .find(|d| d.id == deck)
                .map(|d| (c.channel.as_str(), d))
        })
    }

    #[must_use]
    pub fn find_deck_mut(&mut self, deck: Uuid) -> Option<&mut LightingDeck> {
        self.channel_decks
            .iter_mut()
            .find_map(|c| c.decks.iter_mut().find(|d| d.id == deck))
    }

    /// Drop deck entries for channels that no longer exist.
    ///
    /// Called after a channel is removed, so a deleted channel does not leave lighting decks
    /// contributing from nowhere.
    pub fn retain_channels(&mut self, existing: &[String]) {
        self.channel_decks.retain(|c| existing.contains(&c.channel));
    }

    /// True when the show carries no lighting content at all, which is the default.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.looks.is_empty()
            && self.palettes.is_empty()
            && self.channel_decks.iter().all(|c| c.decks.is_empty())
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
    fn decks_mut_creates_then_reuses_a_channel_entry() {
        let mut show = LightingShow::default();
        let channel = "abc12345";
        show.decks_mut(channel)
            .push(LightingDeck::new(Look::new("l")));
        show.decks_mut(channel)
            .push(LightingDeck::new(Look::new("l")));
        assert_eq!(show.channel_decks.len(), 1, "one entry per channel");
        assert_eq!(show.decks(channel).len(), 2);
    }

    #[test]
    fn decks_for_an_unknown_channel_is_empty() {
        let show = LightingShow::default();
        assert!(show.decks("nosuchch").is_empty());
    }

    #[test]
    fn find_deck_reports_its_channel() {
        let mut show = LightingShow::default();
        let channel = "abc12345";
        let deck = LightingDeck::new(Look::new("l"));
        let deck_id = deck.id;
        show.decks_mut(channel).push(deck);
        let (found_channel, found) = show.find_deck(deck_id).expect("found");
        assert_eq!(found_channel, channel);
        assert_eq!(found.id, deck_id);
    }

    #[test]
    fn find_deck_mut_allows_an_edit() {
        let mut show = LightingShow::default();
        let channel = "abc12345";
        let deck = LightingDeck::new(Look::new("l"));
        let deck_id = deck.id;
        show.decks_mut(channel).push(deck);
        show.find_deck_mut(deck_id).unwrap().level = 0.25;
        assert!((show.decks(channel)[0].level - 0.25).abs() < f32::EPSILON);
    }

    /// A deleted channel must not leave lighting decks contributing from nowhere.
    #[test]
    fn retain_channels_drops_orphaned_decks() {
        let mut show = LightingShow::default();
        let kept = "aaaa1111";
        let removed = "bbbb2222";
        show.decks_mut(kept).push(LightingDeck::new(Look::new("l")));
        show.decks_mut(removed)
            .push(LightingDeck::new(Look::new("l")));
        show.retain_channels(&[kept.to_string()]);
        assert_eq!(show.channel_decks.len(), 1);
        assert_eq!(show.decks(kept).len(), 1);
        assert!(show.decks(removed).is_empty());
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
        let channel = "abc12345";
        show.decks_mut(channel)
            .push(LightingDeck::new(Look::new("Deep Blue")));
        let text = serde_json::to_string(&show).unwrap();
        let back: LightingShow = serde_json::from_str(&text).unwrap();
        assert_eq!(show, back);
    }

    /// The invariant the whole change exists for: no two decks share mutable content.
    ///
    /// Before decks owned their content, `LightingDeck { look: Uuid }` meant two decks made from
    /// one saved Look pointed at one object, so editing either changed both — silently, with
    /// nothing in the UI saying they were linked. No video deck behaves that way.
    /// See /spec/lighting-routing.md § A deck owns its content.
    #[test]
    fn two_decks_from_one_saved_look_are_independent() {
        let saved = Look::new("Wash");

        // What `AddLightingDeck` does: a copy under a fresh id, twice.
        let mut a = saved.clone();
        a.id = Uuid::new_v4();
        let mut b = saved.clone();
        b.id = Uuid::new_v4();

        let mut show = LightingShow::default();
        show.looks.push(saved);
        show.decks_mut("ch-1").push(LightingDeck::new(a));
        show.decks_mut("ch-2").push(LightingDeck::new(b));

        let first = show.decks("ch-1")[0].content.id;
        let second = show.decks("ch-2")[0].content.id;
        assert_ne!(first, second, "two decks must not share one content id");

        // Editing one must not reach the other, or the library entry.
        show.look_mut(first).unwrap().name = "Edited".into();
        assert_eq!(show.decks("ch-2")[0].content.name, "Wash");
        assert_eq!(show.looks[0].name, "Wash");
    }

    /// Creating a deck must add nothing to the library. Creating a video deck does not create a
    /// deck preset; this used to push an untitled "Lighting" entry on every blank deck.
    #[test]
    fn a_blank_deck_adds_nothing_to_the_library() {
        let mut show = LightingShow::default();
        show.decks_mut("ch-1")
            .push(LightingDeck::new(Look::new("Lighting")));
        show.decks_mut("ch-1")
            .push(LightingDeck::new(Look::new("Lighting")));
        assert!(
            show.looks.is_empty(),
            "creating decks filled the library: {:?}",
            show.looks
        );
    }

    /// Deck content is addressable by the same `look()` resolver the editor uses, so every
    /// existing command keeps working against a deck's own content.
    #[test]
    fn deck_content_resolves_through_the_look_lookup() {
        let mut show = LightingShow::default();
        let content = Look::new("Wash");
        let id = content.id;
        show.decks_mut("ch-1").push(LightingDeck::new(content));
        assert_eq!(show.look(id).unwrap().name, "Wash");
        show.look_mut(id).unwrap().name = "Edited".into();
        assert_eq!(show.decks("ch-1")[0].content.name, "Edited");
    }
}
