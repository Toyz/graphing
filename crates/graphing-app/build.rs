//! Gathers what graphing is built from for the Open Source section in
//! settings: every crate the `graphing` binary depends on for this platform
//! (from `cargo metadata`, offline), with its version, license, repository
//! and license files, plus graphing's own license and commit. Shipping each
//! crate's license text and where its source lives is what MIT, Apache,
//! BSD and an unmodified MPL-2.0 crate (mp4parse) ask of a binary.
//! Anything that can't be read leaves the list empty rather than failing the build.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

#[allow(dead_code)]
#[path = "src/license.rs"]
mod license;

/// License files larger than this are left out.
const MAX_TEXT: u64 = 256 * 1024;

/// Licenses known to be fine; anything else that isn't refused warns, so
/// a new kind of license gets a look before it ships.
const KNOWN: [&str; 15] = [
    "MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Zlib", "0BSD", "Unlicense", "BSL-1.0", "CC0-1.0", "Unicode-3.0",
    "Unicode-DFS-2016", "MPL-2.0", "LLVM-exception", "bzip2-1.0.6",
];

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/license.rs");
    // Only files that are there: cargo reruns every build for a missing one
    // (a source tarball has no `.git`; a worktree keeps it elsewhere).
    let watched = [root.join("Cargo.lock"), root.join("Cargo.toml")].into_iter().chain(git_files(&root));
    for file in watched.filter(|f| f.exists()) {
        println!("cargo:rerun-if-changed={}", file.display());
    }
    let mut texts = Texts::default();
    let (entries, license, commit) = generate(&root, &mut texts).unwrap_or_else(|e| {
        println!("cargo:warning=the list of crates is empty: {e}");
        Default::default()
    });
    // No copyleft: a crate offering only copyleft licenses fails the build.
    let mut refused = Vec::new();
    for e in &entries {
        if let Some(why) = license::refused(&e.license) {
            refused.push(format!("{} {} ({why})", e.name, e.version));
        } else if e.license.is_empty() {
            println!("cargo:warning={} {} names no license: check it before shipping", e.name, e.version);
        } else if !license::license_ids(&license::elected(&e.license)).iter().all(|id| KNOWN.contains(id)) {
            println!("cargo:warning={} {} is licensed {:?}: check it before shipping", e.name, e.version, e.license);
        }
    }
    if !refused.is_empty() {
        panic!("graphing takes no copyleft dependencies (MPL-2.0 aside), but these are:\n  {}", refused.join("\n  "));
    }
    let code = render(&entries, &texts.all, &license, &commit);
    std::fs::write(PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("open_source.rs"), code).unwrap();
    println!("cargo:rerun-if-env-changed=GRAPHING_VERSION");
    println!("cargo:rustc-env=GRAPHING_VERSION={}", version(&root));
}

/// graphing's version, from git rather than Cargo.toml (nobody has to
/// remember to bump it): `GRAPHING_VERSION` when a release build sets it
/// from its tag, else the nearest `v*` tag as `git describe` puts it
/// (`0.2.0-rc.1`, or `0.2.0-rc.1-4-gabc1234` four commits on), else
/// Cargo's with `-dev`.
fn version(root: &Path) -> String {
    if let Ok(v) = std::env::var("GRAPHING_VERSION")
        && !v.trim().is_empty()
    {
        return v.trim().trim_start_matches('v').to_string();
    }
    let described = Command::new("git")
        .args(["describe", "--tags", "--match", "v[0-9]*", "--dirty"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().trim_start_matches('v').to_string());
    described.unwrap_or_else(|| format!("{}-dev", std::env::var("CARGO_PKG_VERSION").unwrap_or_default()))
}

/// Where git keeps what says which commit is checked out: HEAD, the
/// branches, and packed refs. Asked of git, since a worktree keeps its HEAD
/// apart from the branches it shares.
fn git_files(root: &Path) -> Vec<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--git-path", "HEAD", "--git-path", "refs/heads", "--git-path", "refs/tags", "--git-path", "packed-refs"])
        .current_dir(root)
        .output();
    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).lines().map(|l| root.join(l.trim())).collect(),
        _ => Vec::new(),
    }
}

/// License texts, each kept once.
#[derive(Default)]
struct Texts {
    all: Vec<String>,
    index: HashMap<String, usize>,
}

impl Texts {
    /// Where `text` is kept, adding it the first time.
    fn add(&mut self, text: String) -> usize {
        *self.index.entry(text.clone()).or_insert_with(|| {
            self.all.push(text);
            self.all.len() - 1
        })
    }
}

struct Entry {
    name: String,
    version: String,
    license: String,
    repository: String,
    description: String,
    authors: String,
    /// Each license file's name and its text's index.
    texts: Vec<(String, usize)>,
}

/// Every crate the binary is built from, and graphing's own license and commit.
fn generate(root: &Path, texts: &mut Texts) -> Result<(Vec<Entry>, String, String), String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let target = std::env::var("TARGET").map_err(|e| e.to_string())?;
    let output = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--offline", "--filter-platform", &target, "--manifest-path"])
        .arg(root.join("Cargo.toml"))
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    let meta: Value = serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?;
    let list = |v: &Value| v.as_array().cloned().unwrap_or_default();
    let packages: HashMap<String, Value> = list(&meta["packages"]).into_iter().map(|p| (str(&p["id"]), p)).collect();
    let nodes: HashMap<String, Value> = list(&meta["resolve"]["nodes"]).into_iter().map(|n| (str(&n["id"]), n)).collect();
    let members: HashSet<String> = list(&meta["workspace_members"]).iter().map(str).collect();
    let bin = packages.values().find(|p| p["name"] == "graphing-bin").ok_or("no graphing-bin package")?;

    // Everything the binary needs to build and run: normal and build dependencies, not dev ones.
    let mut seen = HashSet::new();
    let mut stack = vec![str(&bin["id"])];
    while let Some(id) = stack.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        for dep in nodes.get(&id).map(|n| list(&n["deps"])).unwrap_or_default() {
            let needed = list(&dep["dep_kinds"]).iter().any(|k| k["kind"].is_null() || k["kind"] == "build");
            if needed {
                stack.push(str(&dep["pkg"]));
            }
        }
    }

    // In a set order, so the same dependencies always build the same list.
    let mut needed: Vec<&Value> = seen.iter().filter(|id| !members.contains(*id)).filter_map(|id| packages.get(id)).collect();
    needed.sort_by_key(|p| (str(&p["name"]).to_lowercase(), str(&p["version"])));
    let entries: Vec<Entry> = needed
        .into_iter()
        .map(|p| {
            let dir = Path::new(p["manifest_path"].as_str().unwrap_or("")).parent().unwrap_or(Path::new("")).to_path_buf();
            let files = license_files(&dir, p["license_file"].as_str());
            // Fonts it bundles under their own license (the crate's covers its code).
            let mut license = str(&p["license"]);
            if files.iter().any(|(name, _)| name.to_uppercase().starts_with("OFL")) && !license.contains("OFL") {
                license += ", fonts OFL-1.1";
            }
            let texts = files.into_iter().map(|(name, text)| (name, texts.add(text))).collect();
            Entry {
                name: str(&p["name"]),
                version: str(&p["version"]),
                license,
                repository: str(&p["repository"]),
                description: str(&p["description"]).split_whitespace().collect::<Vec<_>>().join(" "),
                // Names, without the email addresses.
                authors: list(&p["authors"])
                    .iter()
                    .map(|a| str(a).split('<').next().unwrap_or("").trim().to_owned())
                    .filter(|a| !a.is_empty())
                    .collect::<Vec<_>>()
                    .join(", "),
                texts,
            }
        })
        .collect();

    let commit = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default();
    Ok((entries, str(&bin["license"]), commit))
}

fn str(v: &Value) -> String {
    v.as_str().unwrap_or("").to_owned()
}

/// The crate's license files, by name: the one its manifest names, and any
/// `LICENSE*`, `LICENCE*`, `COPYING*`, `NOTICE*` or `UNLICENSE*` beside it;
/// then, for fonts it bundles, `OFL*` and `FONT_NOTICE*` in `fonts/`.
fn license_files(dir: &Path, named: Option<&str>) -> Vec<(String, String)> {
    let mut paths: Vec<PathBuf> = named.map(|n| dir.join(n)).into_iter().collect();
    let found = |dir: &Path, prefixes: &[&str]| -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
        let mut found: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .filter(|p| {
                let name = p.file_name().map(|n| n.to_string_lossy().to_uppercase()).unwrap_or_default();
                prefixes.iter().any(|prefix| name.starts_with(prefix))
            })
            .collect();
        found.sort();
        found
    };
    paths.extend(found(dir, &["LICENSE", "LICENCE", "COPYING", "NOTICE", "UNLICENSE"]));
    paths.extend(found(&dir.join("fonts"), &["OFL", "FONT_NOTICE"]));
    let mut seen = HashSet::new();
    paths
        .into_iter()
        .filter(|p| seen.insert(p.clone()))
        .filter(|p| std::fs::metadata(p).is_ok_and(|m| m.len() <= MAX_TEXT))
        .filter_map(|p| {
            let text = std::fs::read_to_string(&p).ok()?.trim().replace("\r\n", "\n");
            let name = p.file_name()?.to_string_lossy().into_owned();
            (!text.is_empty()).then_some((name, text))
        })
        .collect()
}

fn render(entries: &[Entry], texts: &[String], license: &str, commit: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "pub static LICENSE: &str = {license:?};");
    let _ = writeln!(out, "pub static COMMIT: &str = {commit:?};");
    out.push_str("pub static TEXTS: &[&str] = &[\n");
    for t in texts {
        let _ = writeln!(out, "    {t:?},");
    }
    out.push_str("];\npub static CRATES: &[Crate] = &[\n");
    for e in entries {
        let _ = writeln!(
            out,
            "    Crate {{ name: {:?}, version: {:?}, license: {:?}, repository: {:?}, description: {:?}, authors: {:?}, texts: &{:?} }},",
            e.name, e.version, e.license, e.repository, e.description, e.authors, e.texts
        );
    }
    out.push_str("];\n");
    out
}
