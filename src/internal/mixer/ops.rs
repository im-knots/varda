//! Operations the mixer answers alone, addressed by UUID: the commands that
//! change a deck, a channel, or an effect chain without touching any device,
//! file, or output. Each resolves its target first and reports an
//! [`UnknownEntity`](crate::engine::value::entity::UnknownEntity) when the UUID
//! names nothing.

use anyhow::{Context as _, Result};

use super::{EffectChain, EffectLocation, Mixer};
use crate::channel::{BlendMode, DeckSlot};
use crate::deck::{ScalingMode, TapSource};
use crate::engine::value::entity::{EffectTarget, Resolved};

/// Clamp to 0.0–1.0, with a fallback for NaN or infinity.
fn sanitize_unit(value: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        fallback
    }
}

impl Mixer {
    /// `(uuid, name)` for every channel, in order: what a tap deck's label reads.
    pub fn channel_labels(&self) -> Vec<(String, String)> {
        self.channels()
            .iter()
            .map(|ch| (ch.uuid().to_string(), ch.name.clone()))
            .collect()
    }

    /// Move a deck, with all its properties, to the end of another channel.
    ///
    /// # Errors
    ///
    /// Returns an error if either UUID names nothing.
    pub fn move_deck(&mut self, deck_uuid: &str, dst_channel_uuid: &str) -> Result<()> {
        let (src_ch, src_deck) = self.resolve_deck(deck_uuid)?;
        let dst_ch = self.resolve_channel(dst_channel_uuid)?;
        if src_ch == dst_ch {
            return Ok(());
        }
        let channels = self.channels_mut();
        let Some(slot) = channels[src_ch].remove_deck_slot(src_deck) else {
            anyhow::bail!("deck '{deck_uuid}' vanished during move");
        };
        channels[dst_ch].add_deck_slot(slot);
        log::info!("Moved deck {deck_uuid} from ch{src_ch} to ch{dst_ch}");
        Ok(())
    }

    /// Move the deck at `from_idx` to `to_idx` within one channel.
    ///
    /// # Errors
    ///
    /// Returns an error if the channel UUID names nothing or either position is
    /// out of range.
    pub fn reorder_deck(
        &mut self,
        channel_uuid: &str,
        from_idx: usize,
        to_idx: usize,
    ) -> Result<()> {
        let channel_idx = self.resolve_channel(channel_uuid)?;
        if from_idx == to_idx {
            return Ok(());
        }
        let channel = self.channel_mut(channel_idx).context("Invalid channel")?;
        if from_idx >= channel.decks.len() || to_idx >= channel.decks.len() {
            anyhow::bail!(
                "reorder_deck: ordinals {from_idx}->{to_idx} out of range for {} decks",
                channel.decks.len()
            );
        }
        let slot = channel.decks.remove(from_idx);
        channel.decks.insert(to_idx, slot);
        log::info!("Reordered deck in ch {channel_uuid}: {from_idx} -> {to_idx}");
        Ok(())
    }

    /// Remove a deck and every modulation assignment on it or its effects.
    /// Returns the removed slot, so the caller can release whatever device the
    /// deck held.
    ///
    /// # Errors
    ///
    /// Returns an error if the UUID names no deck.
    pub fn remove_deck(&mut self, deck_uuid: &str) -> Resolved<DeckSlot> {
        let (channel_idx, deck_idx) = self.resolve_deck(deck_uuid)?;
        let slot = self.channels_mut()[channel_idx].decks.remove(deck_idx);
        log::info!("Removed deck {deck_uuid} from channel {channel_idx}");
        let modulation = self.modulation_mut();
        modulation
            .remove_assignments_with_prefix(&crate::engine::value::param::deck_prefix(deck_uuid));
        for effect in &slot.deck.effects {
            modulation.remove_assignments_with_prefix(&crate::engine::value::param::effect_prefix(
                effect.uuid(),
            ));
        }
        Ok(slot)
    }

    /// # Errors
    ///
    /// Returns an error if the UUID names no deck.
    pub fn set_deck_opacity(&mut self, deck_uuid: &str, opacity: f32) -> Resolved<()> {
        let (ch, dk) = self.resolve_deck(deck_uuid)?;
        self.channels_mut()[ch].decks[dk].opacity = sanitize_unit(opacity, 1.0);
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if the UUID names no deck.
    pub fn set_deck_blend_mode(&mut self, deck_uuid: &str, mode: BlendMode) -> Resolved<()> {
        let (ch, dk) = self.resolve_deck(deck_uuid)?;
        self.channels_mut()[ch].decks[dk].blend_mode = mode;
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if the UUID names no deck.
    pub fn set_deck_solo(&mut self, deck_uuid: &str, solo: bool) -> Resolved<()> {
        let (ch, dk) = self.resolve_deck(deck_uuid)?;
        self.channels_mut()[ch].set_deck_solo(dk, solo);
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if the UUID names no deck.
    pub fn set_deck_mute(&mut self, deck_uuid: &str, mute: bool) -> Resolved<()> {
        let (ch, dk) = self.resolve_deck(deck_uuid)?;
        self.channels_mut()[ch].set_deck_mute(dk, mute);
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if the UUID names no deck.
    pub fn set_deck_scaling_mode(&mut self, deck_uuid: &str, mode: ScalingMode) -> Resolved<()> {
        let (ch, dk) = self.resolve_deck(deck_uuid)?;
        self.channels_mut()[ch].decks[dk]
            .deck
            .set_scaling_mode(mode);
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if the UUID names no deck.
    pub fn set_deck_transparent(&mut self, deck_uuid: &str, transparent: bool) -> Resolved<()> {
        let (ch, dk) = self.resolve_deck(deck_uuid)?;
        self.channels_mut()[ch].decks[dk]
            .deck
            .set_transparent(transparent);
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if the UUID names no channel.
    pub fn set_channel_opacity(&mut self, channel_uuid: &str, opacity: f32) -> Resolved<()> {
        let ch = self.resolve_channel(channel_uuid)?;
        self.channels_mut()[ch].opacity = sanitize_unit(opacity, 1.0);
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if the UUID names no channel.
    pub fn set_channel_blend_mode(&mut self, channel_uuid: &str, mode: BlendMode) -> Resolved<()> {
        let ch = self.resolve_channel(channel_uuid)?;
        self.channels_mut()[ch].blend_mode = mode;
        Ok(())
    }

    /// Point a tap deck at a different source.
    ///
    /// # Errors
    ///
    /// Returns an error if the UUID names no deck or the deck is not a tap.
    pub fn set_tap_source(&mut self, deck_uuid: &str, source: TapSource) -> Result<()> {
        let (ch, dk) = self.resolve_deck(deck_uuid)?;
        let labels = self.channel_labels();
        let deck = &mut self.channels_mut()[ch].decks[dk].deck;
        let state = deck
            .tap
            .as_mut()
            .context("Deck is not a tap and has no source to repoint")?;
        state.source = source;
        let label = state.source.label(&labels);
        deck.set_source_name(format!("🔁 {label}"));
        Ok(())
    }

    /// Remove an effect and its modulation assignments. Returns the depth
    /// sensor the owning deck stopped needing, if removing this effect made it
    /// the deck's last depth consumer, so the caller can release it.
    ///
    /// # Errors
    ///
    /// Returns an error if the UUID names no effect.
    pub fn remove_effect(
        &mut self,
        effect_uuid: &str,
    ) -> Resolved<Option<crate::depth::DepthSensorId>> {
        let location = self.resolve_effect(effect_uuid)?;
        let (chain, idx) = self.effect_chain_at_mut(location);
        chain.remove(idx);
        let released = match location {
            EffectLocation::Deck {
                channel_idx: ch,
                deck_idx: dk,
                ..
            } => self
                .channel_mut(ch)
                .and_then(|c| c.decks.get_mut(dk))
                .and_then(|s| s.deck.detach_depth_preprocessor_if_unused()),
            _ => None,
        };
        self.modulation_mut().remove_assignments_with_prefix(
            &crate::engine::value::param::effect_prefix(effect_uuid),
        );
        Ok(released)
    }

    /// # Errors
    ///
    /// Returns an error if the UUID names no effect.
    pub fn toggle_effect(&mut self, effect_uuid: &str) -> Resolved<()> {
        let location = self.resolve_effect(effect_uuid)?;
        let (chain, idx) = self.effect_chain_at_mut(location);
        chain[idx].enabled = !chain[idx].enabled;
        Ok(())
    }

    /// Move the effect at `from_idx` to `to_idx` within one chain.
    ///
    /// # Errors
    ///
    /// Returns an error if the target names nothing or either position is out
    /// of range.
    pub fn move_effect(
        &mut self,
        target: &EffectTarget,
        from_idx: usize,
        to_idx: usize,
    ) -> Result<()> {
        let chain = self.resolve_effect_target(target)?;
        if from_idx == to_idx {
            return Ok(());
        }
        let effects = match chain {
            EffectChain::Deck {
                channel_idx,
                deck_idx,
            } => {
                &mut self.channels_mut()[channel_idx].decks[deck_idx]
                    .deck
                    .effects
            }
            EffectChain::Channel { channel_idx } => &mut self.channels_mut()[channel_idx].effects,
            EffectChain::Master => self.master_effects_mut(),
        };
        if from_idx >= effects.len() || to_idx >= effects.len() {
            anyhow::bail!(
                "move_effect: ordinals {from_idx}->{to_idx} out of range for {} effects",
                effects.len()
            );
        }
        let effect = effects.remove(from_idx);
        effects.insert(to_idx, effect);
        Ok(())
    }

    /// Set a deck's shader parameter.
    ///
    /// # Errors
    ///
    /// Returns an error if the UUID names no deck.
    pub fn set_generator_param(
        &mut self,
        deck_uuid: &str,
        name: &str,
        value: crate::params::ParamValue,
    ) -> Resolved<()> {
        let (ch, dk) = self.resolve_deck(deck_uuid)?;
        self.channels_mut()[ch].decks[dk]
            .deck
            .generator_params
            .set(name, value);
        Ok(())
    }

    /// Set an effect's shader parameter, on any chain.
    ///
    /// # Errors
    ///
    /// Returns an error if the UUID names no effect.
    pub fn set_effect_param(
        &mut self,
        effect_uuid: &str,
        name: &str,
        value: crate::params::ParamValue,
    ) -> Resolved<()> {
        let location = self.resolve_effect(effect_uuid)?;
        if let Some(effect) = self.effect_at_mut(location) {
            effect.params.set(name, value);
        }
        Ok(())
    }

    /// Start an analyzer on a deck.
    ///
    /// # Errors
    ///
    /// Returns an error if the deck does not exist or the analyzer cannot start.
    pub(crate) fn request_analyzer(
        &mut self,
        deck_uuid: &str,
        analyzer_type: &str,
        registry: &crate::analyzer::AnalyzerRegistry,
        options: &serde_json::Value,
    ) -> Result<()> {
        let (ch, dk) = self
            .find_deck_by_uuid(deck_uuid)
            .ok_or_else(|| anyhow::anyhow!("Deck '{deck_uuid}' not found"))?;
        let slot = self
            .channel_mut(ch)
            .and_then(|c| c.decks.get_mut(dk))
            .ok_or_else(|| anyhow::anyhow!("Deck slot not accessible"))?;
        slot.deck
            .analyzers
            .request(analyzer_type, registry, options)
            .ok_or_else(|| anyhow::anyhow!("Failed to start analyzer '{analyzer_type}'"))?;
        Ok(())
    }

    /// Stop an analyzer on a deck. Unknown decks and analyzers are ignored.
    pub fn release_analyzer(&mut self, deck_uuid: &str, analyzer_type: &str) {
        if let Some((ch, dk)) = self.find_deck_by_uuid(deck_uuid)
            && let Some(slot) = self.channel_mut(ch).and_then(|c| c.decks.get_mut(dk))
        {
            slot.deck.analyzers.release(analyzer_type);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::renderer::context::GpuContext;

    fn mixer() -> Option<(GpuContext, Mixer)> {
        let gpu = GpuContext::new_headless().ok()?;
        let mixer = Mixer::new(&gpu, 64, 64).ok()?;
        Some((gpu, mixer))
    }

    fn add_deck(gpu: &GpuContext, mixer: &mut Mixer, ch: usize) -> String {
        let deck = crate::deck::Deck::new_solid_color(gpu, [1.0, 0.0, 0.0, 1.0], 64, 64)
            .expect("solid deck");
        let uuid = deck.uuid().to_string();
        mixer.channel_mut(ch).expect("channel").add_deck(deck);
        uuid
    }

    fn channel_uuid(mixer: &Mixer, idx: usize) -> String {
        mixer.channels()[idx].uuid().to_string()
    }

    #[test]
    fn move_deck_between_channels() {
        let Some((gpu, mut mixer)) = mixer() else {
            return;
        };
        let deck = add_deck(&gpu, &mut mixer, 0);
        mixer.move_deck(&deck, &channel_uuid(&mixer, 1)).unwrap();
        assert_eq!(mixer.channels()[0].decks.len(), 0);
        assert_eq!(mixer.channels()[1].decks.len(), 1);
    }

    #[test]
    fn move_deck_to_its_own_channel_is_a_noop() {
        let Some((gpu, mut mixer)) = mixer() else {
            return;
        };
        let deck = add_deck(&gpu, &mut mixer, 0);
        mixer.move_deck(&deck, &channel_uuid(&mixer, 0)).unwrap();
        assert_eq!(mixer.channels()[0].decks.len(), 1);
    }

    #[test]
    fn move_deck_with_an_unknown_uuid_errors_and_moves_nothing() {
        let Some((gpu, mut mixer)) = mixer() else {
            return;
        };
        let deck = add_deck(&gpu, &mut mixer, 0);
        assert!(
            mixer
                .move_deck("nosuchdk", &channel_uuid(&mixer, 0))
                .is_err()
        );
        assert!(mixer.move_deck(&deck, "nosuchch").is_err());
        assert_eq!(mixer.channels()[0].decks.len(), 1);
    }

    #[test]
    fn reorder_deck_within_a_channel() {
        let Some((gpu, mut mixer)) = mixer() else {
            return;
        };
        let first = add_deck(&gpu, &mut mixer, 0);
        add_deck(&gpu, &mut mixer, 0);
        add_deck(&gpu, &mut mixer, 0);
        let ch = channel_uuid(&mixer, 0);
        mixer.reorder_deck(&ch, 0, 2).unwrap();
        assert_eq!(mixer.channels()[0].decks[2].deck.uuid(), first);
        mixer.reorder_deck(&ch, 1, 1).unwrap();
        assert!(mixer.reorder_deck(&ch, 0, 9).is_err(), "out of range");
        assert!(mixer.reorder_deck("nosuchch", 0, 1).is_err());
    }

    #[test]
    fn deck_setters_reject_unknown_and_channel_uuids() {
        let Some((_gpu, mut mixer)) = mixer() else {
            return;
        };
        let ch0 = channel_uuid(&mixer, 0);
        assert!(mixer.set_deck_opacity("nosuchdk", 0.5).is_err());
        assert!(
            mixer.set_deck_opacity(&ch0, 0.5).is_err(),
            "a channel UUID does not name a deck"
        );
        assert!(
            mixer
                .set_deck_blend_mode("nosuchdk", BlendMode::Add)
                .is_err()
        );
        assert!(mixer.set_deck_solo("nosuchdk", true).is_err());
        assert!(mixer.set_deck_mute("nosuchdk", true).is_err());
    }

    #[test]
    fn set_channel_opacity_clamps() {
        let Some((_gpu, mut mixer)) = mixer() else {
            return;
        };
        let ch0 = channel_uuid(&mixer, 0);
        mixer.set_channel_opacity(&ch0, 2.0).unwrap();
        assert!((mixer.channels()[0].opacity - 1.0).abs() < 1e-5);
        mixer.set_channel_opacity(&ch0, -1.0).unwrap();
        assert!(mixer.channels()[0].opacity.abs() < 1e-5);
        mixer.set_channel_opacity(&ch0, f32::NAN).unwrap();
        assert!(
            (mixer.channels()[0].opacity - 1.0).abs() < 1e-5,
            "NaN falls back to 1"
        );
    }

    #[test]
    fn remove_deck_takes_its_modulation_with_it() {
        let Some((gpu, mut mixer)) = mixer() else {
            return;
        };
        let deck = add_deck(&gpu, &mut mixer, 0);
        let source = mixer
            .modulation_mut()
            .add_source(crate::modulation::ModulationSource::sine_lfo(1.0));
        let key = crate::engine::value::param::ParamAddress::deck(
            &deck,
            crate::engine::value::param::DeckTarget::Opacity,
        )
        .to_string();
        mixer.modulation_mut().assign(&key, &source, 1.0, None);

        let slot = mixer.remove_deck(&deck).unwrap();
        assert_eq!(slot.deck.uuid(), deck);
        assert!(mixer.channels()[0].decks.is_empty());
        assert!(!mixer.modulation().has_modulation(&key));
        assert!(mixer.remove_deck(&deck).is_err(), "already gone");
    }

    #[test]
    fn toggle_effect_with_an_unknown_uuid_errors() {
        let Some((_gpu, mut mixer)) = mixer() else {
            return;
        };
        assert!(mixer.toggle_effect("nosuchfx").is_err());
    }
}
