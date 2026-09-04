//! The right-click menu that copies and pastes scene objects.
//!
//! One helper for every surface that shows one, so a deck card in the mixer, a
//! lane header on the timeline, and an effect card in the bottom bar all offer
//! the same three items in the same order and disable them for the same
//! reasons. See /spec/clipboard.md § UI surface.

use crate::engine::{ClipboardKind, ClipboardSource, ClipboardSummary, EngineCommand, PasteTarget};
use crate::usecases::ui::{UIActions, UIData};

/// What a menu was opened on.
pub(crate) struct Subject {
    pub source: ClipboardSource,
    /// What this object is, which is what Copy and Duplicate act on.
    pub kind: ClipboardKind,
    /// Where a paste from this menu lands: directly after the subject, or
    /// inside it when the subject is a container.
    pub paste_target: PasteTarget,
    pub label: String,
}

impl Subject {
    pub fn deck(uuid: &str, label: impl Into<String>) -> Self {
        Self {
            source: ClipboardSource::Deck(uuid.to_string()),
            kind: ClipboardKind::Deck,
            paste_target: PasteTarget::AfterDeck(uuid.to_string()),
            label: label.into(),
        }
    }

    pub fn effect(uuid: &str, label: impl Into<String>) -> Self {
        Self {
            source: ClipboardSource::Effect(uuid.to_string()),
            kind: ClipboardKind::Effect,
            paste_target: PasteTarget::AfterEffect(uuid.to_string()),
            label: label.into(),
        }
    }

    pub fn channel(uuid: &str, label: impl Into<String>) -> Self {
        Self {
            source: ClipboardSource::Channel(uuid.to_string()),
            kind: ClipboardKind::Channel,
            paste_target: PasteTarget::IntoChannel(uuid.to_string()),
            label: label.into(),
        }
    }
}

/// Copy, Duplicate, and Paste, in that order. Callers add a separator and their
/// own items after these.
pub(crate) fn items(ui: &mut egui::Ui, data: &UIData, actions: &mut UIActions, subject: &Subject) {
    let own = noun(subject.kind);
    if ui
        .button(format!("Copy {own} '{}'", subject.label))
        .clicked()
    {
        actions.commands.push(EngineCommand::Copy {
            source: subject.source.clone(),
            // In the mixer a deck is a source; on the timeline it is a source
            // and a placement.
            include_arrangement: data.arrangement_mode_open,
        });
        ui.close();
    }
    if ui.button(format!("Duplicate {own}")).clicked() {
        actions.commands.push(EngineCommand::Duplicate {
            source: subject.source.clone(),
        });
        ui.close();
    }

    let held = data.clipboard.as_ref();
    let target = held.and_then(|summary| paste_target(subject, summary));
    let paste = ui
        .add_enabled(
            target.is_some(),
            egui::Button::new(held.map_or_else(
                || "Paste".to_string(),
                |summary| format!("Paste {} '{}'", noun(summary.kind), summary.label),
            )),
        )
        .on_disabled_hover_text(held.map_or_else(
            || "Copy something first".to_string(),
            |summary| format!("A {} does not go here", noun(summary.kind)),
        ));
    if let Some(target) = target
        && paste.clicked()
    {
        actions.commands.push(EngineCommand::Paste { target });
        ui.close();
    }
}

/// `Cmd+C`, `Cmd+V`, and `Cmd+D` on whatever the bottom bar is following.
///
/// The selection is the subject because it is the one object the UI already
/// knows the user is working on. With nothing selected the shortcuts do
/// nothing, which is quieter than guessing.
pub(crate) fn shortcut(action: crate::keymap::ActionId, data: &UIData, actions: &mut UIActions) {
    let Some(subject) = selection(data) else {
        return;
    };
    match action {
        crate::keymap::ActionId::Copy => actions.commands.push(EngineCommand::Copy {
            source: subject.source,
            include_arrangement: data.arrangement_mode_open,
        }),
        crate::keymap::ActionId::Duplicate => actions.commands.push(EngineCommand::Duplicate {
            source: subject.source,
        }),
        crate::keymap::ActionId::Paste => {
            if let Some(target) = data
                .clipboard
                .as_ref()
                .and_then(|held| paste_target(&subject, held))
            {
                actions.commands.push(EngineCommand::Paste { target });
            }
        }
        _ => {}
    }
}

/// The deck, channel, or master chain the bottom bar is following. A deck wins
/// over its channel, because selecting a deck also leaves its channel selected.
fn selection(data: &UIData) -> Option<Subject> {
    if let Some((ch_idx, deck_idx)) = data.selected_deck {
        let deck = data.channels.get(ch_idx)?.decks.get(deck_idx)?;
        return Some(Subject::deck(&deck.uuid, &deck.name));
    }
    let ch_idx = data.selected_channel?;
    let channel = data.channels.get(ch_idx)?;
    Some(Subject::channel(&channel.uuid, &channel.name))
}

/// Where what is held would land in this menu, or `None` when it does not
/// belong here at all.
///
/// A channel's menu is the one that takes two kinds: a held deck lands in it,
/// and a held channel lands beside it as a new one.
fn paste_target(subject: &Subject, held: &ClipboardSummary) -> Option<PasteTarget> {
    match (subject.kind, held.kind) {
        (ClipboardKind::Channel, ClipboardKind::Channel) => Some(PasteTarget::NewChannel),
        (ClipboardKind::Channel | ClipboardKind::Deck, ClipboardKind::Deck)
        | (ClipboardKind::Effect, ClipboardKind::Effect) => Some(subject.paste_target.clone()),
        _ => None,
    }
}

fn noun(kind: ClipboardKind) -> &'static str {
    match kind {
        ClipboardKind::Deck => "deck",
        ClipboardKind::Channel => "channel",
        ClipboardKind::Effect => "effect",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn holding(kind: ClipboardKind) -> ClipboardSummary {
        ClipboardSummary {
            kind,
            label: "thing".to_string(),
        }
    }

    #[test]
    fn a_deck_menu_takes_a_deck_and_nothing_else() {
        let subject = Subject::deck("deck0001", "waves");
        assert_eq!(
            paste_target(&subject, &holding(ClipboardKind::Deck)),
            Some(PasteTarget::AfterDeck("deck0001".to_string())),
            "a copy lands below the deck whose menu is open"
        );
        assert_eq!(
            paste_target(&subject, &holding(ClipboardKind::Effect)),
            None
        );
        assert_eq!(
            paste_target(&subject, &holding(ClipboardKind::Channel)),
            None
        );
    }

    /// A channel is the one place both a deck and a channel can land, so the
    /// target depends on what is held rather than on what was clicked.
    #[test]
    fn a_channel_menu_takes_both_and_sends_them_to_different_places() {
        let subject = Subject::channel("chan0001", "Ch 0");
        assert_eq!(
            paste_target(&subject, &holding(ClipboardKind::Deck)),
            Some(PasteTarget::IntoChannel("chan0001".to_string()))
        );
        assert_eq!(
            paste_target(&subject, &holding(ClipboardKind::Channel)),
            Some(PasteTarget::NewChannel)
        );
        assert_eq!(
            paste_target(&subject, &holding(ClipboardKind::Effect)),
            None
        );
    }

    /// The fixture selects deck (0,0), which is what the bottom bar is showing,
    /// so that is what `Cmd+C` means.
    #[test]
    fn the_shortcut_acts_on_the_selected_deck() {
        let mut data = UIData::test_fixture();
        data.arrangement_mode_open = false;
        let mut actions = UIActions::default();

        shortcut(crate::keymap::ActionId::Copy, &data, &mut actions);

        let deck = &data.channels[0].decks[0];
        assert!(matches!(
            actions.commands.first(),
            Some(EngineCommand::Copy { source, include_arrangement: false })
                if *source == ClipboardSource::Deck(deck.uuid.clone())
        ));
    }

    /// A copy made on the timeline is a source and a placement; the same copy
    /// made in the mixer is only a source.
    #[test]
    fn the_shortcut_carries_regions_only_in_arrangement_mode() {
        let mut data = UIData::test_fixture();
        data.arrangement_mode_open = true;
        let mut actions = UIActions::default();

        shortcut(crate::keymap::ActionId::Copy, &data, &mut actions);

        assert!(matches!(
            actions.commands.first(),
            Some(EngineCommand::Copy {
                include_arrangement: true,
                ..
            })
        ));
    }

    /// A channel is the subject when no deck is selected, rather than nothing.
    #[test]
    fn the_shortcut_falls_back_to_the_selected_channel() {
        let mut data = UIData::test_fixture();
        data.selected_deck = None;
        data.selected_channel = Some(1);
        let mut actions = UIActions::default();

        shortcut(crate::keymap::ActionId::Duplicate, &data, &mut actions);

        let channel = &data.channels[1];
        assert!(matches!(
            actions.commands.first(),
            Some(EngineCommand::Duplicate { source })
                if *source == ClipboardSource::Channel(channel.uuid.clone())
        ));
    }

    /// With nothing selected the shortcut does nothing, which is quieter than
    /// guessing at a subject.
    #[test]
    fn the_shortcut_does_nothing_without_a_selection() {
        let mut data = UIData::test_fixture();
        data.selected_deck = None;
        data.selected_channel = None;
        let mut actions = UIActions::default();

        shortcut(crate::keymap::ActionId::Copy, &data, &mut actions);

        assert!(actions.commands.is_empty());
    }

    /// Pasting a deck onto a deck selection lands it below that deck, and a
    /// paste of something that does not fit is dropped rather than guessed at.
    #[test]
    fn the_paste_shortcut_respects_what_is_held() {
        let mut data = UIData::test_fixture();
        data.clipboard = Some(holding(ClipboardKind::Effect));
        let mut actions = UIActions::default();
        shortcut(crate::keymap::ActionId::Paste, &data, &mut actions);
        assert!(
            actions.commands.is_empty(),
            "an effect does not land on a deck"
        );

        data.clipboard = Some(holding(ClipboardKind::Deck));
        shortcut(crate::keymap::ActionId::Paste, &data, &mut actions);
        let deck = &data.channels[0].decks[0];
        assert!(matches!(
            actions.commands.first(),
            Some(EngineCommand::Paste { target })
                if *target == PasteTarget::AfterDeck(deck.uuid.clone())
        ));
    }

    #[test]
    fn an_effect_menu_pastes_into_its_own_chain() {
        let subject = Subject::effect("fx000001", "blur");
        assert_eq!(
            paste_target(&subject, &holding(ClipboardKind::Effect)),
            Some(PasteTarget::AfterEffect("fx000001".to_string()))
        );
        assert_eq!(paste_target(&subject, &holding(ClipboardKind::Deck)), None);
    }
}
