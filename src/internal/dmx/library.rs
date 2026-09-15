//! Fixture profile library: resolves a profile reference to a loaded definition.
//!
//! Search order is **user directories first, then bundled**, so a workspace definition always
//! overrides a shipped one. An operator who needs to correct a channel map for tonight's rig
//! should not have to wait for a release. See /spec/dmx-output.md Open Question 1.

use super::ofl;
use super::patch::ProfileSource;
use super::profile::{FixtureProfile, ProfileError};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A profile reference is `vendor/model`, matching Open Fixture Library's layout on disk.
///
/// Resolved against each search directory in turn as `<dir>/<vendor>/<model>.json`.
pub struct ProfileLibrary {
    dirs: Vec<PathBuf>,
    cache: HashMap<String, Arc<FixtureProfile>>,
}

impl ProfileLibrary {
    /// Build a library searching `dirs` in order.
    #[must_use]
    pub fn new(dirs: Vec<PathBuf>) -> Self {
        Self {
            dirs,
            cache: HashMap::new(),
        }
    }

    /// Directories searched, in precedence order.
    #[must_use]
    pub fn dirs(&self) -> &[PathBuf] {
        &self.dirs
    }

    /// Drop cached profiles so edited definitions are picked up on the next patch.
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    /// Resolve a reference to a file path, searching in precedence order.
    fn locate(&self, reference: &str) -> Option<PathBuf> {
        // Reject traversal outright. A profile reference reaches this from a scene file that may
        // have come from someone else, and `../../../etc/passwd` should not be loadable even
        // though the parser would reject the contents.
        if reference.contains("..") || reference.starts_with('/') || reference.contains('\\') {
            return None;
        }
        for dir in &self.dirs {
            let candidate = dir.join(format!("{reference}.json"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    }

    /// Index every available profile's mode names, for the patch editor's mode picker.
    ///
    /// Built once rather than per frame or per patch edit: it parses the whole bundled library
    /// (653 definitions, tens of milliseconds) and the result is shared by `Arc`, so a consumer
    /// reading it every frame costs a refcount bump.
    ///
    /// Reads only what the picker needs. A definition that fails to parse, or is a redirect
    /// stub, is skipped: the picker offers what can actually be patched.
    #[must_use]
    pub fn mode_index(&self) -> std::collections::HashMap<String, Vec<String>> {
        let mut out = std::collections::HashMap::new();
        for reference in self.available() {
            let Some(path) = self.locate(&reference) else {
                continue;
            };
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
                continue;
            };
            let modes: Vec<String> = value
                .get("modes")
                .and_then(serde_json::Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|m| m.get("name").and_then(serde_json::Value::as_str))
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            if !modes.is_empty() {
                out.insert(reference, modes);
            }
        }
        out
    }

    /// Enumerate every profile reference available, sorted, for the patch editor's picker.
    #[must_use]
    pub fn available(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for dir in &self.dirs {
            collect_refs(dir, dir, &mut out);
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

fn collect_refs(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_refs(root, &path, out);
        } else if path.extension().is_some_and(|e| e == "json")
            && let Ok(rel) = path.strip_prefix(root)
            && let Some(text) = rel.with_extension("").to_str()
        {
            out.push(text.replace('\\', "/"));
        }
    }
}

impl ProfileSource for ProfileLibrary {
    fn load(&mut self, reference: &str) -> Result<Arc<FixtureProfile>, ProfileError> {
        if let Some(hit) = self.cache.get(reference) {
            return Ok(Arc::clone(hit));
        }
        let path = self.locate(reference).ok_or_else(|| {
            ProfileError::Io(format!(
                "profile {reference:?} not found in {:?}",
                self.dirs
            ))
        })?;
        let profile = Arc::new(ofl::load(&path)?);
        self.cache
            .insert(reference.to_owned(), Arc::clone(&profile));
        Ok(profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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

    fn write(dir: &Path, reference: &str, body: &str) {
        let path = dir.join(format!("{reference}.json"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    #[test]
    fn loads_a_profile_by_reference() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "generic/rgbw-par", RGBW);
        let mut lib = ProfileLibrary::new(vec![tmp.path().to_path_buf()]);
        let p = lib.load("generic/rgbw-par").expect("loads");
        assert_eq!(p.model, "RGBW Par");
        assert_eq!(p.vendor, "generic", "vendor comes from the directory");
    }

    #[test]
    fn a_missing_profile_names_the_reference_and_the_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let mut lib = ProfileLibrary::new(vec![tmp.path().to_path_buf()]);
        let err = lib.load("nope/missing").unwrap_err().to_string();
        assert!(err.contains("nope/missing"), "{err}");
    }

    /// An operator correcting a channel map for tonight's rig must not have to wait for a
    /// release, so a workspace definition wins over a bundled one.
    #[test]
    fn an_earlier_directory_overrides_a_later_one() {
        let user = tempfile::tempdir().unwrap();
        let bundled = tempfile::tempdir().unwrap();
        write(bundled.path(), "generic/par", RGBW);
        write(
            user.path(),
            "generic/par",
            &RGBW.replace("RGBW Par", "Corrected Par"),
        );
        let mut lib = ProfileLibrary::new(vec![
            user.path().to_path_buf(),
            bundled.path().to_path_buf(),
        ]);
        assert_eq!(lib.load("generic/par").unwrap().model, "Corrected Par");
    }

    #[test]
    fn falls_through_to_a_later_directory() {
        let user = tempfile::tempdir().unwrap();
        let bundled = tempfile::tempdir().unwrap();
        write(bundled.path(), "generic/par", RGBW);
        let mut lib = ProfileLibrary::new(vec![
            user.path().to_path_buf(),
            bundled.path().to_path_buf(),
        ]);
        assert_eq!(lib.load("generic/par").unwrap().model, "RGBW Par");
    }

    #[test]
    fn profiles_are_cached() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "generic/par", RGBW);
        let mut lib = ProfileLibrary::new(vec![tmp.path().to_path_buf()]);
        let a = lib.load("generic/par").unwrap();
        // Remove the file: a cached profile must still resolve.
        fs::remove_file(tmp.path().join("generic/par.json")).unwrap();
        let b = lib.load("generic/par").unwrap();
        assert!(Arc::ptr_eq(&a, &b), "the same Arc must come back");
    }

    #[test]
    fn clearing_the_cache_picks_up_an_edit() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "generic/par", RGBW);
        let mut lib = ProfileLibrary::new(vec![tmp.path().to_path_buf()]);
        assert_eq!(lib.load("generic/par").unwrap().model, "RGBW Par");
        write(
            tmp.path(),
            "generic/par",
            &RGBW.replace("RGBW Par", "Edited"),
        );
        lib.clear_cache();
        assert_eq!(lib.load("generic/par").unwrap().model, "Edited");
    }

    /// A profile reference arrives from a scene file that may have come from someone else.
    #[test]
    fn path_traversal_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let mut lib = ProfileLibrary::new(vec![tmp.path().to_path_buf()]);
        for bad in [
            "../../etc/passwd",
            "generic/../../secret",
            "/etc/passwd",
            "generic\\..\\secret",
        ] {
            assert!(lib.load(bad).is_err(), "{bad} must not resolve");
        }
    }

    #[test]
    fn available_lists_every_reference_sorted() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "zeta/one", RGBW);
        write(tmp.path(), "alpha/two", RGBW);
        write(tmp.path(), "alpha/one", RGBW);
        let lib = ProfileLibrary::new(vec![tmp.path().to_path_buf()]);
        assert_eq!(lib.available(), vec!["alpha/one", "alpha/two", "zeta/one"]);
    }

    #[test]
    fn available_deduplicates_across_directories() {
        let user = tempfile::tempdir().unwrap();
        let bundled = tempfile::tempdir().unwrap();
        write(user.path(), "generic/par", RGBW);
        write(bundled.path(), "generic/par", RGBW);
        let lib = ProfileLibrary::new(vec![
            user.path().to_path_buf(),
            bundled.path().to_path_buf(),
        ]);
        assert_eq!(lib.available(), vec!["generic/par"]);
    }

    #[test]
    fn mode_index_lists_the_modes_a_profile_defines() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "generic/par", RGBW);
        write(
            tmp.path(),
            "acme/mover",
            r#"{"name":"Mover","availableChannels":{
                "Pan":{"capability":{"type":"Pan"}}},
                "modes":[{"name":"3ch","channels":["Pan"]},
                         {"name":"7ch","channels":["Pan"]}]}"#,
        );
        let lib = ProfileLibrary::new(vec![tmp.path().to_path_buf()]);
        let idx = lib.mode_index();
        assert_eq!(idx.get("generic/par").unwrap(), &vec!["4ch".to_string()]);
        assert_eq!(
            idx.get("acme/mover").unwrap(),
            &vec!["3ch".to_string(), "7ch".to_string()],
            "order follows the definition, which is the order the picker shows"
        );
    }

    /// The picker must offer only what can actually be patched.
    #[test]
    fn mode_index_skips_definitions_that_cannot_be_patched() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "bad/broken", "{not json");
        write(
            tmp.path(),
            "bad/modeless",
            r#"{"name":"X","availableChannels":{},"modes":[]}"#,
        );
        let lib = ProfileLibrary::new(vec![tmp.path().to_path_buf()]);
        assert!(lib.mode_index().is_empty());
    }

    #[test]
    fn a_missing_directory_is_not_an_error() {
        let lib = ProfileLibrary::new(vec![PathBuf::from("/definitely/not/here")]);
        assert!(lib.available().is_empty());
    }
}
