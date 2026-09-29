//! The system font database, shared by SVG decks and text decks.
//!
//! Scanning fonts can take hundreds of milliseconds, so [`warm`] starts it on a
//! background thread. The render thread calls [`ready`], which never waits;
//! loader threads call [`database`], which does.
//!
//! A machine with no fonts (a headless Linux container) gets egui's bundled fonts.

use std::collections::BTreeSet;
use std::sync::{Arc, OnceLock};

pub use usvg::fontdb::Database;

/// The generic family text falls back to when its own is not installed.
pub const FALLBACK_FAMILY: &str = "sans-serif";

struct Fonts {
    db: Arc<Database>,
    families: Arc<[String]>,
}

static FONTS: OnceLock<Fonts> = OnceLock::new();

fn load() -> Fonts {
    let started = std::time::Instant::now();
    let mut db = Database::new();
    db.load_system_fonts();
    if db.is_empty() {
        log::warn!("No system fonts found; text draws in the bundled fallback fonts");
        add_bundled(&mut db);
    }
    let families = family_names(&db);
    log::info!(
        "Loaded {} font faces in {} families ({} ms)",
        db.len(),
        families.len(),
        started.elapsed().as_millis()
    );
    Fonts {
        db: Arc::new(db),
        families,
    }
}

/// Start loading the database on a background thread, if nothing has yet.
pub fn warm() {
    if FONTS.get().is_some() {
        return;
    }
    if let Err(e) = std::thread::Builder::new()
        .name("font-scan".into())
        .spawn(|| {
            FONTS.get_or_init(load);
        })
    {
        log::warn!("Could not start the font scan thread: {e}");
    }
}

/// The database, loading it on this thread if no scan has finished. Only for
/// threads that may block: loaders, tests, the CLI.
pub fn database() -> Arc<Database> {
    FONTS.get_or_init(load).db.clone()
}

/// The database if it has finished loading. Never waits.
pub fn ready() -> Option<Arc<Database>> {
    FONTS.get().map(|f| f.db.clone())
}

/// Installed family names, sorted, once the database has loaded.
pub fn families() -> Option<Arc<[String]>> {
    FONTS.get().map(|f| f.families.clone())
}

/// Whether `family` is installed, ignoring case.
pub fn has_family(families: &[String], family: &str) -> bool {
    families.iter().any(|f| f.eq_ignore_ascii_case(family))
}

/// Add the fonts egui bundles, and point the generic families at them.
pub fn add_bundled(db: &mut Database) {
    db.load_font_data(epaint_default_fonts::UBUNTU_LIGHT.to_vec());
    db.load_font_data(epaint_default_fonts::HACK_REGULAR.to_vec());
    db.load_font_data(epaint_default_fonts::NOTO_EMOJI_REGULAR.to_vec());
    db.set_sans_serif_family("Ubuntu");
    db.set_serif_family("Ubuntu");
    db.set_monospace_family("Hack");
}

/// Every family name in `db`, sorted and deduplicated. Only a face's first
/// (primary) family name is used.
pub fn family_names(db: &Database) -> Arc<[String]> {
    db.faces()
        .filter_map(|face| face.families.first().map(|(name, _)| name.clone()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_fonts_give_an_empty_machine_something_to_draw_with() {
        let mut db = Database::new();
        assert!(db.is_empty());
        add_bundled(&mut db);
        let families = family_names(&db);
        assert!(has_family(&families, "ubuntu"), "{families:?}");
        assert!(has_family(&families, "Hack"), "{families:?}");
        let sans = db.query(&usvg::fontdb::Query {
            families: &[usvg::fontdb::Family::SansSerif],
            ..Default::default()
        });
        assert!(sans.is_some(), "sans-serif resolves to a bundled face");
    }

    #[test]
    fn family_names_are_sorted_and_unique() {
        let mut db = Database::new();
        add_bundled(&mut db);
        add_bundled(&mut db);
        let families = family_names(&db);
        let mut sorted = families.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(&*families, sorted.as_slice());
    }
}
