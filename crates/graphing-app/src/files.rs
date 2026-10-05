//! Reading and writing diagram files: plain `.gph` text, or a `.gphz`
//! package holding the text and its images.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

pub type Assets = BTreeMap<String, Vec<u8>>;

/// Whether `path` is a package by its extension.
pub fn is_package_path(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("gphz"))
}

/// The diagram text and its packaged assets (empty for plain `.gph`).
pub fn load(path: &Path) -> io::Result<(String, Assets)> {
    let bytes = std::fs::read(path)?;
    if graphing_package::is_package(&bytes) {
        let pkg = graphing_package::read(&bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        return Ok((pkg.doc, pkg.assets));
    }
    let text = String::from_utf8(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    Ok((text, Assets::new()))
}

/// Write atomically: a package for `.gphz`, plain text otherwise.
/// A package also takes a fresh snapshot of every diagram the text links
/// to, into `assets`, so it still shows them where those files are not.
pub fn save(path: &Path, text: &str, assets: &mut Assets) -> io::Result<()> {
    let bytes = if is_package_path(path) {
        graphing_export::snapshot_links(text, path.parent(), assets);
        graphing_package::write(&graphing_package::Package { doc: text.to_string(), meta: String::new(), assets: assets.clone() })
    } else {
        text.as_bytes().to_vec()
    };
    let tmp = path.with_extension("save.tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_and_package_round_trip() {
        let dir = std::env::temp_dir().join(format!("graphing-files-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let plain = dir.join("a.gph");
        save(&plain, "a -> b\n", &mut Assets::new()).unwrap();
        assert_eq!(load(&plain).unwrap(), ("a -> b\n".to_string(), Assets::new()));

        let pkg = dir.join("a.gphz");
        let mut assets = Assets::new();
        assets.insert("logo-12345678.png".into(), b"\x89PNG fake".to_vec());
        let text = "logo: image { src: \"asset:logo-12345678.png\" }\n";
        save(&pkg, text, &mut assets.clone()).unwrap();
        assert_eq!(load(&pkg).unwrap(), (text.to_string(), assets));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
