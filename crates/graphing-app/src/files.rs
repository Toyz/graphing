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

/// Whether `path` is a graphing diagram (`.gph` or `.gphz`) by its extension.
pub fn is_diagram_path(path: &Path) -> bool {
    is_package_path(path) || path.extension().is_some_and(|e| e.eq_ignore_ascii_case("gph"))
}

/// Whether graphing opens or imports `path`, by its extension.
pub fn is_openable(path: &Path) -> bool {
    is_diagram_path(path) || graphing_import::Format::of_path(path).is_some()
}

/// The diagram text and its packaged assets (empty for plain `.gph`).
/// Line endings come back as `\n`, so edits splice one kind of line;
/// [`save`] puts `\r\n` back for a file that had it.
pub fn load(path: &Path) -> io::Result<(String, Assets)> {
    let pkg = graphing_package::open(std::fs::read(path)?).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok((pkg.doc, pkg.assets))
}

/// Whether the file at `path` ends its first line with `\r\n`.
fn uses_crlf(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 4096];
    let Ok(n) = std::fs::File::open(path).and_then(|mut f| f.read(&mut head)) else { return false };
    head[..n].iter().position(|&b| b == b'\n').is_some_and(|i| i > 0 && head[i - 1] == b'\r')
}

/// Write atomically: a package for `.gphz`, plain text otherwise.
/// A package also takes a fresh snapshot of every diagram the text links
/// to, into `assets`, so it still shows them where those files are not.
pub fn save(path: &Path, text: &str, assets: &mut Assets) -> io::Result<()> {
    let bytes = if is_package_path(path) {
        graphing_export::snapshot_links(text, path.parent(), assets);
        graphing_package::write(&graphing_package::Package { doc: text.to_string(), meta: String::new(), assets: assets.clone() })
    } else if uses_crlf(path) {
        text.replace("\r\n", "\n").replace('\n', "\r\n").into_bytes()
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
    fn crlf_files_edit_as_lf_and_keep_their_endings() {
        let dir = std::env::temp_dir().join(format!("graphing-crlf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("w.gph");
        std::fs::write(&file, "a -> b\r\n# note\r\n").unwrap();
        let (text, _) = load(&file).unwrap();
        assert_eq!(text, "a -> b\n# note\n");
        save(&file, &format!("{text}c\n"), &mut Assets::new()).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"a -> b\r\n# note\r\nc\r\n");
        // A new file is written with plain `\n`.
        let fresh = dir.join("n.gph");
        save(&fresh, "a\n", &mut Assets::new()).unwrap();
        assert_eq!(std::fs::read(&fresh).unwrap(), b"a\n");
        std::fs::remove_dir_all(&dir).ok();
    }

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
