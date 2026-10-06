//! What graphing is built from, gathered at build time by `build.rs`, for
//! the Open Source section in settings: the crates compiled into it, each
//! with its license, source and license texts.

/// A crate compiled into graphing.
pub struct Crate {
    pub name: &'static str,
    pub version: &'static str,
    /// Its license as an SPDX expression, like `MIT OR Apache-2.0`.
    pub license: &'static str,
    pub repository: &'static str,
    pub description: &'static str,
    /// Who wrote it, as its manifest says.
    pub authors: &'static str,
    /// Its license files: each one's name, and its text as an index into
    /// [`TEXTS`], where each text is kept once. Empty for the many crates
    /// whose package ships none.
    pub texts: &'static [(&'static str, usize)],
}

#[allow(dead_code)]
mod generated {
    use super::Crate;
    include!(concat!(env!("OUT_DIR"), "/open_source.rs"));
}
pub use generated::*;

/// graphing's version, from its git tag (see build.rs).
pub const VERSION: &str = env!("GRAPHING_VERSION");

pub use crate::license::{elected, license_ids};

/// Each crate's elected license (see [`elected`]), in [`CRATES`] order, worked out once.
pub fn families() -> &'static [String] {
    static FAMILIES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    FAMILIES.get_or_init(|| CRATES.iter().map(|c| elected(c.license)).collect())
}

/// The crate's license files that apply under `license` (its elected one):
/// those named for it (`LICENSE-MIT` for MIT) and plain `LICENSE`/`NOTICE`
/// files; all of them when none are named for it.
pub fn texts_for(c: &Crate, license: &str) -> Vec<(&'static str, &'static str)> {
    let stems: Vec<String> = license_ids(license).iter().map(|id| id.split(['-', '.']).next().unwrap_or(id).to_ascii_uppercase()).collect();
    let all: Vec<(&'static str, &'static str)> = c.texts.iter().map(|&(name, i)| (name, TEXTS[i])).collect();
    let applies = |name: &str| {
        let upper = name.to_ascii_uppercase();
        let plain = matches!(upper.split('.').next(), Some("LICENSE" | "LICENCE" | "COPYING" | "NOTICE"));
        plain || stems.iter().any(|s| upper.contains(s.as_str()))
    };
    let picked: Vec<_> = all.iter().copied().filter(|(name, _)| applies(name)).collect();
    if picked.is_empty() { all } else { picked }
}

/// Elected licenses and how many crates use each, most used first.
pub fn family_counts() -> Vec<(&'static str, usize)> {
    let mut counts: Vec<(&'static str, usize)> = Vec::new();
    for family in families() {
        match counts.iter_mut().find(|(f, _)| *f == family) {
            Some((_, n)) => *n += 1,
            None => counts.push((family, 1)),
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    counts
}

/// The summary's entry for the licenses it doesn't name (see [`named_families`]).
pub const OTHER: &str = "Other";

/// Whether `family` (an elected license) asks more of software built with
/// it than keeping its notice: MPL's changed files published, (L)GPL's and
/// the like's terms passed on. Such a license is always named on the Open
/// Source section, however few crates it covers.
pub fn has_conditions(family: &str) -> bool {
    const WITH_CONDITIONS: [&str; 8] = ["MPL", "LGPL", "GPL", "AGPL", "EPL", "CDDL", "EUPL", "CC-BY-SA"];
    license_ids(family).iter().any(|id| {
        let id = id.to_ascii_uppercase();
        WITH_CONDITIONS.iter().any(|c| id.starts_with(c))
    })
}

/// The families the Open Source section names, of `counts` (most used first):
/// the `most` used, and every one with conditions (see [`has_conditions`]),
/// so none is lost among the rest, which go together as [`OTHER`].
pub fn named_families(counts: &[(&'static str, usize)], most: usize) -> Vec<&'static str> {
    counts.iter().enumerate().filter(|&(i, &(f, _))| i < most || has_conditions(f)).map(|(_, &(f, _))| f).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_license_with_conditions_is_named_however_few_crates_use_it() {
        assert!(has_conditions("MPL-2.0") && has_conditions("LGPL-2.1-or-later") && has_conditions("GPL-3.0-only"));
        assert!(!has_conditions("MIT") && !has_conditions("Apache-2.0 WITH LLVM-exception") && !has_conditions("CC0-1.0"));
        let counts = [("MIT", 500), ("Apache-2.0", 40), ("Unicode-3.0", 27), ("BSD-3-Clause", 5), ("BSD-2-Clause", 4), ("Zlib", 3), ("MPL-2.0", 1), ("CC0-1.0", 1)];
        assert_eq!(named_families(&counts, 5), ["MIT", "Apache-2.0", "Unicode-3.0", "BSD-3-Clause", "BSD-2-Clause", "MPL-2.0"]);
    }

    #[test]
    fn the_list_has_graphings_dependencies_and_their_texts() {
        // Offline builds without metadata get no crates; normal ones do.
        if CRATES.is_empty() {
            return;
        }
        let serde = CRATES.iter().find(|c| c.name == "serde").expect("serde is a dependency");
        assert!(!serde.texts.is_empty(), "serde ships its license files");
        assert!(CRATES.iter().all(|c| c.texts.iter().all(|&(_, i)| i < TEXTS.len())));
        // Offered MIT or Apache-2.0, it's taken as MIT, and shows MIT's file.
        assert_eq!(texts_for(serde, "MIT").iter().map(|(name, _)| *name).collect::<Vec<_>>(), ["LICENSE-MIT"]);
        // The one MPL crate is listed with its text and where its source is.
        let mp4 = CRATES.iter().find(|c| c.name == "mp4parse").expect("mp4parse is a dependency");
        assert_eq!(elected(mp4.license), "MPL-2.0");
        assert!(!mp4.repository.is_empty() && !mp4.texts.is_empty());
        assert!(!CRATES.iter().any(|c| c.name.starts_with("graphing")), "workspace crates aren't listed");
        assert!(CRATES.iter().all(|c| crate::license::refused(c.license).is_none()), "no copyleft gets in");
        assert_eq!(family_counts().iter().map(|(_, n)| n).sum::<usize>(), CRATES.len());
    }

    #[test]
    fn choices_take_the_most_permissive_option() {
        assert_eq!(elected("MIT OR Apache-2.0"), "MIT");
        assert_eq!(elected("Apache-2.0 OR MIT"), "MIT");
        assert_eq!(elected("MIT/Apache-2.0"), "MIT");
        assert_eq!(elected("Apache-2.0 OR GPL-2.0-only"), "Apache-2.0");
        assert_eq!(elected("LGPL-2.1-or-later OR MPL-2.0"), "MPL-2.0");
        assert_eq!(elected("Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT"), "MIT");
        assert_eq!(elected("(MIT OR Apache-2.0) AND Unicode-3.0"), "MIT AND Unicode-3.0");
        assert_eq!(elected("Apache-2.0 WITH LLVM-exception"), "Apache-2.0 WITH LLVM-exception");
        assert_eq!(elected("(broken"), "(broken");
        assert_eq!(elected(""), "Unknown");
    }
}
