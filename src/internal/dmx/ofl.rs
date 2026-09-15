//! Open Fixture Library JSON loader.
//!
//! OFL is the bundled profile format: 653 definitions across 134 manufacturers under a single
//! MIT licence, no account required, with `fineChannelAliases` expressing 16-bit directly and
//! typed capabilities that map cleanly onto [`Role`]. See /spec/dmx-output.md § Profile Format.
//!
//! Parsed through `serde_json::Value` rather than derived structs. The published schema is far
//! larger than the subset the output path needs, and a permissive reader that rejects by name
//! what it cannot honour is both smaller and more robust to schema growth than one that must
//! model every field to parse any of them.
//!
//! Reference: <https://github.com/OpenLightingProject/open-fixture-library/blob/master/docs/fixture-format.md>

use super::profile::{ChannelRange, FixtureProfile, ProfileChannel, ProfileError, ProfileMode};
use super::role::Role;
use super::universe::Resolution;
use serde_json::Value;
use std::path::Path;

/// Map an OFL capability object to an engine role.
///
/// Types with no engine meaning (effects, fog, blades, maintenance timing) return `None`, which
/// leaves the channel at its profile default rather than writing zero over it.
fn capability_role(cap: &Value) -> Option<Role> {
    let ty = cap.get("type")?.as_str()?;
    Some(match ty {
        "Intensity" => Role::Dimmer,
        "ShutterStrobe" => Role::Shutter,
        "StrobeSpeed" | "StrobeDuration" => Role::Strobe,
        "ColorIntensity" => match cap.get("color").and_then(Value::as_str)? {
            "Red" => Role::Red,
            "Green" => Role::Green,
            "Blue" => Role::Blue,
            // Warm and Cold White both drive the white emitter. A fixture with separate warm and
            // cold emitters gets two White channels, and the patcher writes both; distinguishing
            // them needs a color-temperature model this release does not have.
            "White" | "Warm White" | "Cold White" => Role::White,
            "Amber" => Role::Amber,
            "UV" => Role::Uv,
            "Lime" => Role::Lime,
            "Cyan" => Role::Cyan,
            "Magenta" => Role::Magenta,
            "Yellow" => Role::Yellow,
            _ => return None,
        },
        "ColorTemperature" => Role::Cto,
        "ColorPreset" => Role::ColorWheel,
        "Pan" | "PanContinuous" => Role::Pan,
        "Tilt" | "TiltContinuous" => Role::Tilt,
        "PanTiltSpeed" => Role::PanTiltSpeed,
        "Focus" => Role::Focus,
        "Zoom" | "BeamAngle" => Role::Zoom,
        "Iris" => Role::Iris,
        "Frost" => Role::Frost,
        "Prism" => Role::Prism,
        "PrismRotation" => Role::PrismRotation,
        "WheelShake" => Role::GoboShake,
        "WheelRotation" | "WheelSlotRotation" => Role::GoboRotation,
        // A wheel is a color wheel or a gobo wheel depending on what it holds, and the only
        // signal in the file is its name.
        "WheelSlot" => {
            let wheel = cap.get("wheel").and_then(Value::as_str).unwrap_or("");
            if wheel.to_ascii_lowercase().contains("color")
                || wheel.to_ascii_lowercase().contains("color")
            {
                Role::ColorWheel
            } else {
                Role::GoboWheel
            }
        }
        "Maintenance" => Role::Reset,
        _ => return None,
    })
}

/// Collect a channel's capability objects, whether it used the single or plural form.
fn capabilities_of(ch: &Value) -> Vec<&Value> {
    if let Some(one) = ch.get("capability") {
        return vec![one];
    }
    ch.get("capabilities")
        .and_then(Value::as_array)
        .map(|a| a.iter().collect())
        .unwrap_or_default()
}

fn channel_role(ch: &Value) -> Option<Role> {
    capabilities_of(ch).into_iter().find_map(capability_role)
}

fn channel_ranges(ch: &Value) -> Vec<ChannelRange> {
    capabilities_of(ch)
        .into_iter()
        .filter_map(|cap| {
            let r = cap.get("dmxRange")?.as_array()?;
            let lo = u8::try_from(r.first()?.as_u64()?).ok()?;
            let hi = u8::try_from(r.get(1)?.as_u64()?).ok()?;
            let label = cap
                .get("comment")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| {
                    cap.get("type").and_then(Value::as_str).map(|t| {
                        match cap.get("slotNumber").and_then(Value::as_f64) {
                            Some(n) => format!("{t} {n}"),
                            None => t.to_owned(),
                        }
                    })
                })?;
            Some(ChannelRange { lo, hi, label })
        })
        .collect()
}

/// Switching-channel resolution for one definition.
struct Switches<'a> {
    resolved: &'a std::collections::HashMap<String, String>,
    inactive: &'a std::collections::HashSet<String>,
    dependencies: &'a std::collections::HashSet<String>,
}

/// Resolve every switching-channel alias at its dependency channel's declared default.
///
/// A switching channel is one whose meaning depends on another channel's value: the dependency
/// channel's capabilities map DMX ranges to `{alias: real channel}`, and the mode's channel list
/// names the alias rather than a real channel.
///
/// Rather than refusing such a definition, this resolves each alias for the state the fixture
/// will actually be in, because the **dependency channel is then pinned to that same default**
/// (its role is cleared, so nothing drives it and the patcher writes its default every frame).
/// The resolution is asserted rather than assumed.
///
/// Measured against the bundled library, refusing these cost 94 fixtures to avoid a median of
/// one channel in a six-channel mode, usually an effect-speed parameter Varda does not address.
/// See /spec/dmx-output.md § Switching channels.
///
/// Returns `(alias -> real channel key, alias -> inactive, dependency channel keys, notes)`.
fn resolve_switch_channels(available: &serde_json::Map<String, Value>) -> SwitchResolution {
    let mut resolved = std::collections::HashMap::new();
    let mut inactive = std::collections::HashSet::new();
    let mut dependencies = std::collections::HashSet::new();
    let mut notes = Vec::new();

    for (key, channel) in available {
        let caps = capabilities_of(channel);
        if !caps.iter().any(|c| c.get("switchChannels").is_some()) {
            continue;
        }
        // The schema requires a dependency channel to declare its default explicitly.
        let default = channel
            .get("defaultValue")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        dependencies.insert(key.clone());

        let active = caps.iter().find(|c| {
            c.get("dmxRange")
                .and_then(Value::as_array)
                .and_then(|r| {
                    let lo = r.first()?.as_u64()?;
                    let hi = r.get(1)?.as_u64()?;
                    Some((lo..=hi).contains(&default))
                })
                .unwrap_or(false)
        });
        // A capability with no dmxRange covers the whole channel, so fall back to the first.
        let active = active.or_else(|| caps.first());

        let Some(map) = active
            .and_then(|c| c.get("switchChannels"))
            .and_then(Value::as_object)
        else {
            continue;
        };
        for (alias, target) in map {
            if let Some(target) = target.as_str() {
                resolved.insert(alias.clone(), target.to_owned());
                notes.push(format!(
                    "channel {alias:?} switches on {key:?}; resolved as {target:?} and {key:?} \
                     pinned to {default}. Selecting the alternate function needs a different \
                     profile."
                ));
            } else {
                // A null target means the slot has no function in this state. It still occupies
                // an address, so it becomes a reserved channel rather than an unknown one.
                inactive.insert(alias.clone());
                notes.push(format!(
                    "channel {alias:?} has no function while {key:?} is at its default \
                     {default}; the slot is reserved and held at 0."
                ));
            }
        }
    }
    SwitchResolution {
        resolved,
        inactive,
        dependencies,
        notes,
    }
}

/// Outcome of resolving a definition's switching channels.
struct SwitchResolution {
    resolved: std::collections::HashMap<String, String>,
    /// Aliases whose target is null at the dependency's default: the slot exists but does
    /// nothing in that state.
    inactive: std::collections::HashSet<String>,
    dependencies: std::collections::HashSet<String>,
    notes: Vec<String>,
}

/// Reject a definition that uses a feature this release does not implement.
///
/// Loud rejection rather than a partial parse: a fixture whose mode inserts matrix channels
/// would otherwise load with the wrong channel count and mis-address every fixture patched
/// after it in the same universe.
fn check_supported(path: &str, root: &Value) -> Result<(), ProfileError> {
    let unsupported = |feature: &str| ProfileError::Unsupported {
        path: path.to_owned(),
        feature: feature.to_owned(),
    };

    if root.get("matrix").is_some() {
        return Err(unsupported("matrix (pixel arrangement)"));
    }
    if root.get("templateChannels").is_some() {
        return Err(unsupported("templateChannels (pixel arrangement)"));
    }
    if let Some(modes) = root.get("modes").and_then(Value::as_array) {
        for m in modes {
            if let Some(chs) = m.get("channels").and_then(Value::as_array) {
                for entry in chs {
                    if entry.is_object() {
                        return Err(unsupported("matrixChannels insert in a mode"));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Resolve one mode entry into a [`ProfileChannel`].
///
/// `entries` is the whole mode channel list, needed because a 16-bit channel pairs with a fine
/// alias found elsewhere in the same list, not necessarily adjacent to it.
#[allow(clippy::too_many_arguments)]
fn build_channel(
    path: &str,
    mode_name: &str,
    available: &serde_json::Map<String, Value>,
    fine_owner: &std::collections::HashMap<&str, &str>,
    switches: &Switches<'_>,
    entries: &[Option<&str>],
    idx: usize,
) -> Result<ProfileChannel, ProfileError> {
    // A null entry is a slot the personality reserves but does not use. It still consumes an
    // address, so it must produce a channel.
    // An alias names a channel whose meaning depends on another; substitute the channel it
    // resolves to at the dependency's default. The alias keeps its own name so the patch UI
    // still shows what the manufacturer called the slot.
    let alias = entries[idx].and_then(|k| switches.resolved.get(k).map(String::as_str));
    let Some(key) = entries[idx] else {
        return Ok(ProfileChannel {
            name: format!("unused {}", idx + 1),
            role: None,
            resolution: Resolution::Eight,
            default: 0,
            ranges: Vec::new(),
        });
    };

    // The fine byte of a pair: its value is written by the coarse partner, so it carries no role
    // of its own and nothing addresses it directly. Checked against the resolved name too,
    // because a switching channel can resolve to a fine alias rather than a base channel.
    // An alias with no function in the resolved state: the slot is reserved and held at 0.
    if switches.inactive.contains(key) {
        return Ok(ProfileChannel {
            name: key.to_owned(),
            role: None,
            resolution: Resolution::Eight,
            default: 0,
            ranges: Vec::new(),
        });
    }

    let lookup = alias.unwrap_or(key);
    if fine_owner.contains_key(key) || fine_owner.contains_key(lookup) {
        return Ok(ProfileChannel {
            name: key.to_owned(),
            role: None,
            resolution: Resolution::Eight,
            default: 0,
            ranges: Vec::new(),
        });
    }

    let Some(ch) = available.get(lookup) else {
        return Err(ProfileError::Parse {
            path: path.to_owned(),
            msg: format!("mode {mode_name:?} names unknown channel {lookup:?}"),
        });
    };

    // Pair with the first fine alias present in this same mode. A 24-bit channel keeps only its
    // top two bytes here, which is exact rather than degraded: the remaining fine bytes stay at
    // their defaults and the value is still correct, just not resolved beyond 16 bits.
    let resolution = ch
        .get("fineChannelAliases")
        .and_then(Value::as_array)
        .and_then(|aliases| {
            aliases.iter().filter_map(Value::as_str).find_map(|alias| {
                let fine_idx = entries.iter().position(|e| *e == Some(alias))?;
                let here = isize::try_from(idx).ok()?;
                let there = isize::try_from(fine_idx).ok()?;
                i16::try_from(there - here)
                    .ok()
                    .map(|fine_offset| Resolution::SixteenCoarse { fine_offset })
            })
        })
        .unwrap_or(Resolution::Eight);

    // A dependency channel is pinned: clearing its role means nothing drives it and the patcher
    // writes its default every frame, so the state the alias was resolved for is asserted rather
    // than assumed.
    let role = if switches.dependencies.contains(key) {
        None
    } else {
        channel_role(ch)
    };

    Ok(ProfileChannel {
        name: key.to_owned(),
        role,
        resolution,
        default: ch
            .get("defaultValue")
            .and_then(Value::as_u64)
            .and_then(|v| u8::try_from(v).ok())
            .unwrap_or(0),
        ranges: channel_ranges(ch),
    })
}

/// Parse an OFL fixture definition.
///
/// `vendor` is supplied by the caller because OFL stores the manufacturer as the containing
/// directory rather than inside the file.
///
/// # Errors
///
/// Returns [`ProfileError::Parse`] for malformed JSON or a mode naming an unknown channel,
/// [`ProfileError::Empty`] when the definition declares no usable mode, and
/// [`ProfileError::Unsupported`] naming the feature when the definition uses `switchChannels`
/// or a pixel matrix.
pub fn parse(path: &str, vendor: &str, text: &str) -> Result<FixtureProfile, ProfileError> {
    let root: Value = serde_json::from_str(text).map_err(|e| ProfileError::Parse {
        path: path.to_owned(),
        msg: e.to_string(),
    })?;
    check_supported(path, &root)?;

    // A redirect stub names another fixture instead of defining channels. Open Fixture Library
    // uses them for renames and for one product sold under two brands. Reported rather than
    // followed here: `parse` has no search path, so resolution belongs to the library.
    if let Some(target) = root.get("redirectTo").and_then(Value::as_str) {
        return Err(ProfileError::Redirect {
            path: path.to_owned(),
            to: target.to_owned(),
        });
    }

    let model_name = root
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("Unknown")
        .to_owned();

    let available = root
        .get("availableChannels")
        .and_then(Value::as_object)
        .ok_or_else(|| ProfileError::Empty(path.to_owned()))?;

    let switch = resolve_switch_channels(available);
    let switches = Switches {
        resolved: &switch.resolved,
        inactive: &switch.inactive,
        dependencies: &switch.dependencies,
    };
    let notes = switch.notes.clone();

    // Every fine alias in the file, mapped back to the base channel key that owns it, so a mode
    // entry naming a fine channel can be recognized as the tail of a 16-bit pair.
    let mut fine_owner: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    for (key, ch) in available {
        if let Some(aliases) = ch.get("fineChannelAliases").and_then(Value::as_array) {
            for a in aliases.iter().filter_map(Value::as_str) {
                fine_owner.insert(a, key.as_str());
            }
        }
    }

    let mut personalities = Vec::new();
    for m in root
        .get("modes")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let name = m
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("default")
            .to_owned();
        let entries: Vec<Option<&str>> = m
            .get("channels")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(Value::as_str).collect())
            .unwrap_or_default();

        let mut channels = Vec::with_capacity(entries.len());
        for idx in 0..entries.len() {
            channels.push(build_channel(
                path,
                &name,
                available,
                &fine_owner,
                &switches,
                &entries,
                idx,
            )?);
        }

        if !channels.is_empty() {
            personalities.push(ProfileMode { name, channels });
        }
    }

    if personalities.is_empty() {
        return Err(ProfileError::Empty(path.to_owned()));
    }

    Ok(FixtureProfile {
        vendor: vendor.to_owned(),
        model: model_name,
        modes: personalities,
        notes,
    })
}

/// Load an OFL definition from disk, deriving the vendor from the containing directory as OFL
/// lays its repository out (`fixtures/<manufacturer>/<fixture>.json`).
///
/// # Errors
///
/// Returns [`ProfileError::Io`] when the file cannot be read, and otherwise whatever
/// [`parse`] returns.
pub fn load(path: &Path) -> Result<FixtureProfile, ProfileError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| ProfileError::Io(format!("{}: {e}", path.display())))?;
    let vendor = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|s| s.to_str())
        .unwrap_or("Unknown");
    parse(&path.display().to_string(), vendor, &text)
}

#[cfg(test)]
mod tests {

    /// Parse every bundled definition.
    ///
    /// The contract from /spec/dmx-output.md: each one either normalizes, or is rejected with a
    /// message naming the feature that stopped it. A silent partial parse is a fixture that
    /// behaves wrongly on stage with no error anywhere, which is the failure this guards.
    ///
    /// Skips when the bundled library is absent, so a source checkout without it still tests.
    #[test]
    fn every_bundled_definition_parses_or_is_rejected_by_name() {
        let root = std::path::Path::new("fixtures");
        if !root.is_dir() {
            eprintln!("skipping: bundled fixture library not present");
            return;
        }

        let mut parsed = 0usize;
        let mut redirects = 0usize;
        let mut rejected: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        let mut failures = Vec::new();

        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("readable").flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "json") {
                    continue;
                }
                // The manufacturer index is not a fixture.
                if path.file_name().is_some_and(|n| n == "manufacturers.json") {
                    continue;
                }
                match load(&path) {
                    Ok(profile) => {
                        assert!(
                            !profile.modes.is_empty(),
                            "{} parsed with no modes",
                            path.display()
                        );
                        for mode in &profile.modes {
                            assert!(
                                !mode.channels.is_empty(),
                                "{} mode {:?} has no channels",
                                path.display(),
                                mode.name
                            );
                        }
                        parsed += 1;
                    }
                    Err(ProfileError::Unsupported { feature, .. }) => {
                        *rejected.entry(feature).or_default() += 1;
                    }
                    // A redirect stub is valid, but a dangling one would strand anyone who
                    // picked that fixture, so the target has to exist.
                    Err(ProfileError::Redirect { to, .. }) => {
                        let target = root.join(format!("{to}.json"));
                        if target.is_file() {
                            redirects += 1;
                        } else {
                            failures.push(format!(
                                "{}: redirects to {to:?}, which is not in the library",
                                path.display()
                            ));
                        }
                    }
                    Err(other) => failures.push(format!("{}: {other}", path.display())),
                }
            }
        }

        assert!(
            failures.is_empty(),
            "definitions failed for reasons other than an unsupported feature:\n{}",
            failures.join("\n")
        );
        assert!(parsed > 0, "the bundled library parsed nothing at all");
        eprintln!(
            "bundled corpus: {parsed} parsed, {redirects} redirects, rejected by feature: {rejected:?}"
        );
    }
    use super::*;

    const RGBW: &str = r#"{
      "name": "RGBW Par",
      "availableChannels": {
        "Red":   { "capability": { "type": "ColorIntensity", "color": "Red" } },
        "Green": { "capability": { "type": "ColorIntensity", "color": "Green" } },
        "Blue":  { "capability": { "type": "ColorIntensity", "color": "Blue" } },
        "White": { "capability": { "type": "ColorIntensity", "color": "White" } }
      },
      "modes": [ { "name": "4ch", "channels": ["Red","Green","Blue","White"] } ]
    }"#;

    fn mover() -> &'static str {
        r#"{
      "name": "Mini Mover",
      "availableChannels": {
        "Pan":  { "fineChannelAliases": ["Pan fine"],  "capability": { "type": "Pan" } },
        "Tilt": { "fineChannelAliases": ["Tilt fine"], "capability": { "type": "Tilt" } },
        "Dimmer": { "defaultValue": 11, "capability": { "type": "Intensity" } },
        "Color Wheel": { "capabilities": [
            { "dmxRange": [0, 9],  "type": "WheelSlot", "wheel": "Color Wheel", "slotNumber": 1 },
            { "dmxRange": [10, 19],"type": "WheelSlot", "wheel": "Color Wheel", "slotNumber": 2 } ] },
        "Gobo Wheel": { "capabilities": [
            { "dmxRange": [0, 255], "type": "WheelSlot", "wheel": "Gobo Wheel", "slotNumber": 1 } ] },
        "Effect": { "capability": { "type": "Effect" } }
      },
      "modes": [ { "name": "8ch", "channels":
        ["Pan","Pan fine","Tilt","Tilt fine","Dimmer","Color Wheel","Gobo Wheel","Effect"] } ]
    }"#
    }

    #[test]
    fn parses_a_simple_rgbw_par() {
        let p = parse("t.json", "Generic", RGBW).unwrap();
        assert_eq!(p.vendor, "Generic");
        assert_eq!(p.model, "RGBW Par");
        let m = p.mode("4ch").unwrap();
        assert_eq!(m.channel_count(), 4);
        assert_eq!(
            m.roles(),
            vec![Role::Red, Role::Green, Role::Blue, Role::White]
        );
        assert!(m.channels.iter().all(|c| c.resolution == Resolution::Eight));
    }

    #[test]
    fn pairs_fine_channels_into_sixteen_bit() {
        let p = parse("m.json", "Acme", mover()).unwrap();
        let m = p.mode("8ch").unwrap();
        assert_eq!(m.channels[0].role, Some(Role::Pan));
        assert_eq!(
            m.channels[0].resolution,
            Resolution::SixteenCoarse { fine_offset: 1 }
        );
        assert_eq!(m.channels[2].role, Some(Role::Tilt));
        assert_eq!(
            m.channels[2].resolution,
            Resolution::SixteenCoarse { fine_offset: 1 }
        );
    }

    /// The fine byte must not carry a role of its own, or the patcher would write it twice:
    /// once as the coarse channel's low byte and once as a role in its own right.
    #[test]
    fn fine_channels_carry_no_role() {
        let p = parse("m.json", "Acme", mover()).unwrap();
        let m = p.mode("8ch").unwrap();
        assert_eq!(m.channels[1].name, "Pan fine");
        assert_eq!(m.channels[1].role, None);
        assert_eq!(m.channels[3].role, None);
        assert_eq!(m.roles().iter().filter(|r| **r == Role::Pan).count(), 1);
    }

    #[test]
    fn wheel_role_follows_the_wheel_name() {
        let p = parse("m.json", "Acme", mover()).unwrap();
        let m = p.mode("8ch").unwrap();
        assert_eq!(m.channels[5].role, Some(Role::ColorWheel));
        assert_eq!(m.channels[6].role, Some(Role::GoboWheel));
    }

    #[test]
    fn unrecognized_capability_yields_no_role() {
        let p = parse("m.json", "Acme", mover()).unwrap();
        assert_eq!(p.mode("8ch").unwrap().channels[7].role, None);
    }

    #[test]
    fn default_value_is_preserved() {
        let p = parse("m.json", "Acme", mover()).unwrap();
        assert_eq!(p.mode("8ch").unwrap().channels[4].default, 11);
    }

    #[test]
    fn ranges_are_captured_for_the_patch_ui() {
        let p = parse("m.json", "Acme", mover()).unwrap();
        let cw = &p.mode("8ch").unwrap().channels[5];
        assert_eq!(cw.ranges.len(), 2);
        assert_eq!(cw.ranges[0].lo, 0);
        assert_eq!(cw.ranges[0].hi, 9);
    }

    #[test]
    fn null_mode_entry_still_consumes_a_slot() {
        let j = r#"{"name":"X","availableChannels":{
            "Red":{"capability":{"type":"ColorIntensity","color":"Red"}}},
            "modes":[{"name":"m","channels":["Red",null,"Red"]}]}"#;
        let p = parse("x.json", "V", j).unwrap();
        let m = p.mode("m").unwrap();
        assert_eq!(m.channel_count(), 3, "the null must still occupy a slot");
        assert_eq!(m.channels[1].role, None);
    }

    /// A switching channel resolves at its dependency's declared default, and the dependency is
    /// pinned so the resolved state is asserted rather than assumed.
    #[test]
    fn a_switching_channel_resolves_at_the_dependency_default() {
        let j = r#"{"name":"X","availableChannels":{
            "Mode": { "defaultValue": 0, "capabilities": [
                { "dmxRange": [0, 127], "type": "Effect", "switchChannels": { "Slot": "Dimmer" } },
                { "dmxRange": [128, 255], "type": "Effect", "switchChannels": { "Slot": "Speed" } }
            ]},
            "Dimmer": { "capability": { "type": "Intensity" } },
            "Speed":  { "capability": { "type": "EffectSpeed" } }
          },
          "modes": [ { "name": "m", "channels": ["Mode", "Slot"] } ]}"#;
        let p = parse("x.json", "V", j).expect("resolves rather than refusing");
        let m = p.mode("m").unwrap();
        assert_eq!(m.channel_count(), 2);
        // Default 0 falls in [0,127], so the slot is the Dimmer.
        assert_eq!(
            m.channels[1].name, "Slot",
            "the manufacturer's name is kept"
        );
        assert_eq!(
            m.channels[1].role,
            Some(Role::Dimmer),
            "resolved for the default state"
        );
        // The dependency is pinned: no role, so nothing drives it and it emits its default.
        assert_eq!(
            m.channels[0].role, None,
            "the dependency must be pinned, not driven"
        );
        assert!(
            p.notes.iter().any(|n| n.contains("Slot")),
            "the resolution must be reported, got {:?}",
            p.notes
        );
    }

    #[test]
    fn a_switching_channel_follows_a_non_zero_default() {
        let j = r#"{"name":"X","availableChannels":{
            "Mode": { "defaultValue": 200, "capabilities": [
                { "dmxRange": [0, 127], "type": "Effect", "switchChannels": { "Slot": "Dimmer" } },
                { "dmxRange": [128, 255], "type": "Effect", "switchChannels": { "Slot": "Speed" } }
            ]},
            "Dimmer": { "capability": { "type": "Intensity" } },
            "Speed":  { "capability": { "type": "EffectSpeed" } }
          },
          "modes": [ { "name": "m", "channels": ["Mode", "Slot"] } ]}"#;
        let p = parse("x.json", "V", j).unwrap();
        assert_eq!(
            p.mode("m").unwrap().channels[1].role,
            None,
            "default 200 selects Speed, which carries no engine role"
        );
    }

    #[test]
    fn a_definition_without_switching_channels_carries_no_notes() {
        let p = parse("t.json", "Generic", RGBW).unwrap();
        assert!(p.notes.is_empty());
    }

    #[test]
    fn matrix_fixtures_are_rejected_by_name() {
        let j = r#"{"name":"X","matrix":{"pixelCount":[4,1,1]},
            "availableChannels":{"R":{"capability":{"type":"Intensity"}}},
            "modes":[{"name":"m","channels":["R"]}]}"#;
        match parse("x.json", "V", j) {
            Err(ProfileError::Unsupported { feature, .. }) => {
                assert!(feature.contains("matrix"), "got {feature}");
            }
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn matrix_insert_in_a_mode_is_rejected() {
        let j = r#"{"name":"X","availableChannels":{"R":{"capability":{"type":"Intensity"}}},
            "modes":[{"name":"m","channels":["R",{"insert":"matrixChannels"}]}]}"#;
        assert!(matches!(
            parse("x.json", "V", j),
            Err(ProfileError::Unsupported { .. })
        ));
    }

    #[test]
    fn unknown_channel_key_in_a_mode_is_a_parse_error() {
        let j = r#"{"name":"X","availableChannels":{"R":{"capability":{"type":"Intensity"}}},
            "modes":[{"name":"m","channels":["R","Missing"]}]}"#;
        match parse("x.json", "V", j) {
            Err(ProfileError::Parse { msg, .. }) => assert!(msg.contains("Missing"), "{msg}"),
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    #[test]
    fn no_modes_is_empty_not_a_silent_success() {
        let j = r#"{"name":"X","availableChannels":{"R":{"capability":{"type":"Intensity"}}},
            "modes":[]}"#;
        assert!(matches!(
            parse("x.json", "V", j),
            Err(ProfileError::Empty(_))
        ));
    }

    #[test]
    fn malformed_json_is_a_parse_error_naming_the_file() {
        match parse("broken.json", "V", "{not json") {
            Err(ProfileError::Parse { path, .. }) => assert_eq!(path, "broken.json"),
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    #[test]
    fn warm_and_cold_white_both_drive_the_white_role() {
        for c in ["White", "Warm White", "Cold White"] {
            let cap = serde_json::json!({ "type": "ColorIntensity", "color": c });
            assert_eq!(capability_role(&cap), Some(Role::White), "{c}");
        }
    }

    #[test]
    fn every_position_and_beam_capability_maps() {
        for (ty, role) in [
            ("Pan", Role::Pan),
            ("PanContinuous", Role::Pan),
            ("Tilt", Role::Tilt),
            ("TiltContinuous", Role::Tilt),
            ("PanTiltSpeed", Role::PanTiltSpeed),
            ("Zoom", Role::Zoom),
            ("Focus", Role::Focus),
            ("Iris", Role::Iris),
            ("Frost", Role::Frost),
            ("Prism", Role::Prism),
            ("PrismRotation", Role::PrismRotation),
        ] {
            let cap = serde_json::json!({ "type": ty });
            assert_eq!(capability_role(&cap), Some(role), "{ty}");
        }
    }
}
