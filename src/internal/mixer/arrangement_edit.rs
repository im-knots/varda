//! Editing the arrangement the mixer plays: lanes, regions, cues, and what the
//! show does when idle. Lanes and regions are ordinary scene data, so every
//! edit here is undoable through the command path. See /spec/arrangement.md.

use super::Mixer;
use crate::arrangement::{Cue, IdleBehaviour, LaneConfig, RegionConfig};

/// Why an arrangement edit was refused.
#[derive(Debug, thiserror::Error)]
pub enum ArrangementError {
    #[error("Deck '{0}' not found")]
    NoSuchDeck(String),
    #[error("No arrangement lane for deck '{0}'")]
    NoSuchLane(String),
    #[error("Region {index} not found on lane '{deck}'")]
    NoSuchRegion { deck: String, index: usize },
    #[error("No cue '{0}'")]
    NoSuchCue(String),
    #[error("a region must end after it starts")]
    InvalidRegion,
    #[error("a cue sits at a position of zero or more")]
    InvalidCuePosition,
}

impl ArrangementError {
    /// Whether the edit named something that does not exist, as opposed to
    /// asking for something malformed.
    pub fn is_not_found(&self) -> bool {
        !matches!(self, Self::InvalidRegion | Self::InvalidCuePosition)
    }
}

type Edit<T> = Result<T, ArrangementError>;

impl Mixer {
    fn lane_mut_or_err(&mut self, deck_uuid: &str) -> Edit<&mut LaneConfig> {
        self.arrangement_mut()
            .lane_mut(deck_uuid)
            .ok_or_else(|| ArrangementError::NoSuchLane(deck_uuid.to_string()))
    }

    /// Add a lane for a deck, or keep the existing one.
    ///
    /// A lane *is* the deck, so this creates no entity and is idempotent: two
    /// callers racing to arrange the same deck must not produce two rows.
    ///
    /// # Errors
    ///
    /// Returns an error if the deck does not exist.
    pub fn add_lane(&mut self, deck_uuid: &str) -> Edit<()> {
        if self.find_deck_by_uuid(deck_uuid).is_none() {
            return Err(ArrangementError::NoSuchDeck(deck_uuid.to_string()));
        }
        let arrangement = self.arrangement_mut();
        if arrangement.lane(deck_uuid).is_none() {
            arrangement.lanes.push(LaneConfig::new(deck_uuid));
        }
        Ok(())
    }

    /// Take a lane and its curves out of the scene, handing the deck back to
    /// Performance mode. Reports whether there was one.
    ///
    /// Also the teardown a deck's own removal runs: a lane is a deck's placement
    /// rather than an object beside it, so a deck that is gone cannot keep one.
    /// An orphan lane draws no row (rows are read from the mixer's decks) but
    /// still saves, and its envelopes still drive a parameter key nothing
    /// answers to.
    pub fn drop_lane(&mut self, deck_uuid: &str) -> bool {
        let Some(envelopes) = self
            .arrangement()
            .and_then(|a| a.lane(deck_uuid))
            .map(|lane| {
                lane.envelope_uuids()
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
        else {
            return false;
        };
        // Envelopes belong to the modulation graph, so removing the row has to
        // take them with it or the deck stays driven by an orphan curve.
        for uuid in &envelopes {
            self.modulation_mut().remove_source(uuid);
        }
        self.arrangement_mut()
            .lanes
            .retain(|l| l.deck_uuid != deck_uuid);
        true
    }

    /// # Errors
    ///
    /// Returns an error if the deck has no lane.
    pub fn remove_lane(&mut self, deck_uuid: &str) -> Edit<()> {
        if self.drop_lane(deck_uuid) {
            Ok(())
        } else {
            Err(ArrangementError::NoSuchLane(deck_uuid.to_string()))
        }
    }

    /// Add a visibility span to a deck's lane, creating the lane if needed.
    /// Returns the region's index.
    ///
    /// # Errors
    ///
    /// Returns an error if the region ends before it starts or the deck does
    /// not exist.
    pub fn add_region(&mut self, deck_uuid: &str, region: RegionConfig) -> Edit<usize> {
        if !region.is_valid() {
            return Err(ArrangementError::InvalidRegion);
        }
        self.add_lane(deck_uuid)?;
        let lane = self.lane_mut_or_err(deck_uuid)?;
        lane.regions.push(region);
        let index = lane.regions.len() - 1;
        self.sync_lane_opacity_envelope(deck_uuid);
        Ok(index)
    }

    /// Replace one region in place, for a move, a resize, or a fade drag.
    ///
    /// # Errors
    ///
    /// Returns an error if the region is malformed or does not exist.
    pub fn update_region(
        &mut self,
        deck_uuid: &str,
        index: usize,
        region: RegionConfig,
    ) -> Edit<()> {
        if !region.is_valid() {
            return Err(ArrangementError::InvalidRegion);
        }
        let slot = self
            .lane_mut_or_err(deck_uuid)?
            .regions
            .get_mut(index)
            .ok_or_else(|| ArrangementError::NoSuchRegion {
                deck: deck_uuid.to_string(),
                index,
            })?;
        *slot = region;
        self.sync_lane_opacity_envelope(deck_uuid);
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if the region does not exist.
    pub fn remove_region(&mut self, deck_uuid: &str, index: usize) -> Edit<()> {
        let lane = self.lane_mut_or_err(deck_uuid)?;
        if index >= lane.regions.len() {
            return Err(ArrangementError::NoSuchRegion {
                deck: deck_uuid.to_string(),
                index,
            });
        }
        lane.regions.remove(index);
        self.sync_lane_opacity_envelope(deck_uuid);
        Ok(())
    }

    /// Fold a lane's automation rows away, or open them again.
    ///
    /// # Errors
    ///
    /// Returns an error if the deck has no lane.
    pub fn set_lane_collapsed(&mut self, deck_uuid: &str, collapsed: bool) -> Edit<()> {
        self.lane_mut_or_err(deck_uuid)?.collapsed = collapsed;
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if the behaviour shows a deck that does not exist.
    pub fn set_idle_behaviour(&mut self, idle: IdleBehaviour) -> Edit<()> {
        if let IdleBehaviour::ShowDeck { deck_uuid } = &idle
            && self.find_deck_by_uuid(deck_uuid).is_none()
        {
            return Err(ArrangementError::NoSuchDeck(deck_uuid.clone()));
        }
        self.arrangement_mut().idle = idle;
        Ok(())
    }

    /// Mark an instant worth returning to. Returns the cue's UUID.
    ///
    /// An empty name is filled in from how many cues exist, so the common case
    /// (drop one and keep working) still produces something the arrows can be
    /// read against.
    ///
    /// # Errors
    ///
    /// Returns an error if the position is negative or not finite.
    pub fn add_cue(&mut self, at: f64, name: &str) -> Edit<String> {
        if !at.is_finite() || at < 0.0 {
            return Err(ArrangementError::InvalidCuePosition);
        }
        let arrangement = self.arrangement_mut();
        let name = if name.is_empty() {
            format!("Cue {}", arrangement.cues.len() + 1)
        } else {
            name.to_string()
        };
        let cue = Cue {
            uuid: crate::ids::generate_short_uuid(),
            name,
            at,
        };
        let uuid = cue.uuid.clone();
        arrangement.add_cue(cue);
        Ok(uuid)
    }

    /// Move or rename a cue. Absent fields are left alone, so a drag does not
    /// have to restate the name.
    ///
    /// # Errors
    ///
    /// Returns an error if the position is invalid or the cue does not exist.
    pub fn update_cue(&mut self, uuid: &str, at: Option<f64>, name: Option<String>) -> Edit<()> {
        if at.is_some_and(|at| !at.is_finite() || at < 0.0) {
            return Err(ArrangementError::InvalidCuePosition);
        }
        let arrangement = self.arrangement_mut();
        let cue = arrangement
            .cue_mut(uuid)
            .ok_or_else(|| ArrangementError::NoSuchCue(uuid.to_string()))?;
        if let Some(at) = at {
            cue.at = at;
        }
        if let Some(name) = name {
            cue.name = name;
        }
        // A move can reorder the list, and navigation reads it in order.
        arrangement.sort_cues();
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if the cue does not exist.
    pub fn remove_cue(&mut self, uuid: &str) -> Edit<()> {
        let arrangement = self.arrangement_mut();
        let before = arrangement.cues.len();
        arrangement.cues.retain(|c| c.uuid != uuid);
        if arrangement.cues.len() == before {
            return Err(ArrangementError::NoSuchCue(uuid.to_string()));
        }
        Ok(())
    }
}
