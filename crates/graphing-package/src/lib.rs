//! `.gphz`: a diagram's `.gph` text and its assets (images) in one file.
//!
//! Not a zip. The layout is a 6-byte header, then one record per entry:
//!
//! ```text
//! magic    b"GPHZ"
//! version  u8 (1)
//! flags    u8 (0)
//! records, to end of file:
//!   kind      u8   0 = document (.gph text), 1 = meta (JSON), 2 = asset
//!   codec     u8   0 = stored, 1 = zstd
//!   name_len  u16  little endian
//!   name      utf-8
//!   raw_len   u32  size after decoding
//!   data_len  u32  size as stored
//!   hash      [u8; 32]  BLAKE3 of the decoded bytes
//!   data      data_len bytes
//! ```
//!
//! Text is zstd-compressed; images already compressed (PNG, JPEG, WebP, GIF)
//! are stored as they are, anything else is compressed when that is smaller.
//! Assets are named by content hash, so the same image is kept once, and the
//! hashes are checked on read. Assets the text no longer references
//! (`asset:<name>`) are dropped on write.
//!
//! Diagrams the text links to (`ref { src: "auth.gph" }`) travel along as
//! snapshots named `linked/<src>`, so a shared package still shows them;
//! they are dropped once no link names their `src`.

use std::collections::BTreeMap;

const MAGIC: &[u8; 4] = b"GPHZ";
const VERSION: u8 = 1;
const HEADER: usize = 6;
const ZSTD_LEVEL: i32 = 12;

/// Prefix for asset references in the `.gph` text: `src: "asset:logo-3f2a9c1d.png"`.
pub const ASSET_PREFIX: &str = "asset:";
/// Name prefix of linked diagram snapshots: `linked/auth.gph`.
pub const LINKED_PREFIX: &str = "linked/";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not a graphing package")]
    NotPackage,
    #[error("package version {0} is newer than this build reads")]
    Version(u8),
    #[error("package is truncated")]
    Truncated,
    #[error("entry `{0}` is damaged (hash mismatch)")]
    Damaged(String),
    #[error("entry `{0}`: {1}")]
    Decode(String, String),
    #[error("package has no document")]
    NoDocument,
    #[error("diagram text is not UTF-8")]
    NotText,
}

/// A diagram with its assets.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Package {
    /// The `.gph` text.
    pub doc: String,
    /// Free-form JSON for things outside the text (thumbnails later, app
    /// version). Empty when unused.
    pub meta: String,
    /// Asset name -> bytes.
    pub assets: BTreeMap<String, Vec<u8>>,
}

impl Package {
    /// Add `bytes` under a content-addressed name derived from `original`
    /// (`logo.png` -> `logo-3f2a9c1d.png`) and return the reference to put in
    /// the text (`asset:logo-3f2a9c1d.png`). The same bytes always get the
    /// same name, so adding twice stores once.
    pub fn add_asset(&mut self, original: &str, bytes: Vec<u8>) -> String {
        let name = asset_name(original, &bytes);
        self.assets.entry(name.clone()).or_insert(bytes);
        format!("{ASSET_PREFIX}{name}")
    }

    /// Bytes for a reference written as `asset:<name>` (or the bare name).
    pub fn asset(&self, reference: &str) -> Option<&[u8]> {
        self.assets.get(reference.strip_prefix(ASSET_PREFIX).unwrap_or(reference)).map(Vec::as_slice)
    }

    /// The snapshot kept of the diagram a link's `src` names.
    pub fn linked(&self, src: &str) -> Option<&[u8]> {
        self.assets.get(&linked_name(src)).map(Vec::as_slice)
    }

    /// Drop assets the text does not mention, and snapshots of diagrams no
    /// link names any more.
    pub fn prune(&mut self) {
        let doc = &self.doc;
        self.assets.retain(|name, _| match name.strip_prefix(LINKED_PREFIX) {
            Some(src) => doc.contains(&format!("\"{src}\"")),
            None => doc.contains(&format!("{ASSET_PREFIX}{name}")),
        });
    }
}

/// `logo.png` + bytes -> `logo-3f2a9c1d.png`: a readable stem, eight hex of
/// the BLAKE3 hash, the original extension.
pub fn asset_name(original: &str, bytes: &[u8]) -> String {
    let file = original.rsplit(['/', '\\']).next().unwrap_or(original);
    let (stem, ext) = match file.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, Some(e.to_lowercase())),
        _ => (file, None),
    };
    let stem: String = stem.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).take(40).collect();
    let stem = if stem.is_empty() { "asset".to_string() } else { stem };
    let hash = blake3::hash(bytes).to_hex();
    match ext {
        Some(e) => format!("{stem}-{}.{e}", &hash[..8]),
        None => format!("{stem}-{}", &hash[..8]),
    }
}

/// The asset name of the snapshot of the diagram a link's `src` names.
pub fn linked_name(src: &str) -> String {
    format!("{LINKED_PREFIX}{src}")
}

/// A `.gph` or `.gphz` file's bytes: a package as it is, plain text as a
/// package with no assets. Line endings come back as `\n`.
pub fn open(bytes: Vec<u8>) -> Result<Package, Error> {
    let mut pkg = if is_package(&bytes) { read(&bytes)? } else { Package { doc: String::from_utf8(bytes).map_err(|_| Error::NotText)?, ..Default::default() } };
    if pkg.doc.contains('\r') {
        pkg.doc = pkg.doc.replace("\r\n", "\n");
    }
    Ok(pkg)
}

/// Whether `bytes` start like a package.
pub fn is_package(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

/// Formats that are compressed already; zstd would only waste time.
fn precompressed(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG")
        || bytes.starts_with(b"\xff\xd8\xff")
        || bytes.starts_with(b"GIF8")
        || (bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP")
        || (bytes.len() > 12 && &bytes[4..8] == b"ftyp")
}

fn encode(bytes: &[u8], try_zstd: bool) -> (u8, Vec<u8>) {
    if try_zstd
        && let Ok(z) = zstd::bulk::compress(bytes, ZSTD_LEVEL)
        && z.len() < bytes.len()
    {
        return (1, z);
    }
    (0, bytes.to_vec())
}

fn record(out: &mut Vec<u8>, kind: u8, name: &str, bytes: &[u8], try_zstd: bool) {
    let (codec, data) = encode(bytes, try_zstd);
    out.push(kind);
    out.push(codec);
    out.extend_from_slice(&(name.len() as u16).to_le_bytes());
    out.extend_from_slice(name.as_bytes());
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(blake3::hash(bytes).as_bytes());
    out.extend_from_slice(&data);
}

/// The package as bytes. Unreferenced assets are left out.
pub fn write(pkg: &Package) -> Vec<u8> {
    let mut pkg = pkg.clone();
    pkg.prune();
    let mut out = Vec::with_capacity(HEADER + pkg.doc.len() / 3 + pkg.assets.values().map(Vec::len).sum::<usize>());
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.push(0);
    record(&mut out, 0, "diagram.gph", pkg.doc.as_bytes(), true);
    if !pkg.meta.is_empty() {
        record(&mut out, 1, "meta.json", pkg.meta.as_bytes(), true);
    }
    for (name, bytes) in &pkg.assets {
        record(&mut out, 2, name, bytes, !precompressed(bytes));
    }
    out
}

/// Read a package, checking every entry's hash.
pub fn read(bytes: &[u8]) -> Result<Package, Error> {
    if !is_package(bytes) || bytes.len() < HEADER {
        return Err(Error::NotPackage);
    }
    if bytes[4] > VERSION {
        return Err(Error::Version(bytes[4]));
    }
    let mut pkg = Package::default();
    let mut doc = None;
    let mut at = HEADER;
    let take = |at: &mut usize, n: usize| -> Result<&[u8], Error> {
        let s = bytes.get(*at..*at + n).ok_or(Error::Truncated)?;
        *at += n;
        Ok(s)
    };
    while at < bytes.len() {
        let kind = take(&mut at, 1)?[0];
        let codec = take(&mut at, 1)?[0];
        let name_len = u16::from_le_bytes(take(&mut at, 2)?.try_into().expect("2 bytes")) as usize;
        let name = String::from_utf8_lossy(take(&mut at, name_len)?).into_owned();
        let raw_len = u32::from_le_bytes(take(&mut at, 4)?.try_into().expect("4 bytes")) as usize;
        let data_len = u32::from_le_bytes(take(&mut at, 4)?.try_into().expect("4 bytes")) as usize;
        let hash: [u8; 32] = take(&mut at, 32)?.try_into().expect("32 bytes");
        let data = take(&mut at, data_len)?;
        let raw = match codec {
            0 => data.to_vec(),
            1 => zstd::bulk::decompress(data, raw_len).map_err(|e| Error::Decode(name.clone(), e.to_string()))?,
            c => return Err(Error::Decode(name, format!("unknown codec {c}"))),
        };
        if raw.len() != raw_len || blake3::hash(&raw).as_bytes() != &hash {
            return Err(Error::Damaged(name));
        }
        match kind {
            0 => doc = Some(String::from_utf8(raw).map_err(|e| Error::Decode(name, e.to_string()))?),
            1 => pkg.meta = String::from_utf8_lossy(&raw).into_owned(),
            2 => {
                pkg.assets.insert(name, raw);
            }
            // Unknown kinds from a newer minor revision are skipped.
            _ => {}
        }
    }
    pkg.doc = doc.ok_or(Error::NoDocument)?;
    Ok(pkg)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png() -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
        v.extend((0..2000u32).map(|i| (i * 7 % 251) as u8));
        v
    }

    #[test]
    fn round_trips_and_dedupes() {
        let mut p = Package { doc: String::new(), ..Default::default() };
        let a = p.add_asset("pics/Logo Final.PNG", png());
        let b = p.add_asset("copy.png", png());
        assert_ne!(a, b, "names keep their stem");
        assert_eq!(p.add_asset("pics/Logo Final.PNG", png()), a, "same file, same name");
        p.doc = format!("img: image {{ src: \"{a}\" }}\nimg2: image {{ src: \"{b}\" }}\n");
        p.meta = "{\"app\":\"graphing\"}".into();
        let bytes = write(&p);
        let back = read(&bytes).unwrap();
        assert_eq!(back, p);
        assert_eq!(back.asset(&a).map(<[u8]>::len), Some(png().len()));
    }

    #[test]
    fn text_compresses_and_unused_assets_go() {
        let doc = "a: rounded \"API Gateway\" { fill: #e7f5ff }\n".repeat(200);
        let mut p = Package { doc: doc.clone(), ..Default::default() };
        p.add_asset("unused.png", png());
        let bytes = write(&p);
        assert!(bytes.len() < doc.len() / 10, "{} vs {}", bytes.len(), doc.len());
        let back = read(&bytes).unwrap();
        assert!(back.assets.is_empty());
        assert_eq!(back.doc, doc);
    }

    #[test]
    fn damage_is_caught() {
        let p = Package { doc: "a -> b\n".into(), ..Default::default() };
        let mut bytes = write(&p);
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        assert!(matches!(read(&bytes), Err(Error::Damaged(_) | Error::Decode(..))));
        assert!(matches!(read(b"PK\x03\x04"), Err(Error::NotPackage)));
        assert!(matches!(read(&bytes[..10]), Err(Error::Truncated)));
    }

    #[test]
    fn names_are_tidy() {
        let n = asset_name("C:\\pics\\My Logo (1).JPG", b"x");
        assert!(n.starts_with("My-Logo--1--") && n.ends_with(".jpg"), "{n}");
        assert!(asset_name("", b"x").starts_with("asset-"));
    }

    #[test]
    fn linked_snapshots_stay_while_a_link_names_them() {
        let mut pkg = Package { doc: "a: ref { src: \"auth.gph\" }\n".into(), ..Default::default() };
        pkg.assets.insert(format!("{LINKED_PREFIX}auth.gph"), b"auth".to_vec());
        pkg.assets.insert(format!("{LINKED_PREFIX}old.gph"), b"old".to_vec());
        let back = read(&write(&pkg)).unwrap();
        assert_eq!(back.linked("auth.gph"), Some(&b"auth"[..]));
        assert_eq!(back.linked("old.gph"), None, "no link names it any more");
    }
}
