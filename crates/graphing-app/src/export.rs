//! File-level export and import used by the workspace commands.

use std::path::Path;

use graphing_export::{SvgOptions, Theme, scene_of, to_png, to_svg};

/// Write `src` as `ext`: `svg`, `png` or `sysml` (SysML v2 text).
/// `base` is the folder of the diagram's own file, for its links.
pub fn write(src: &str, path: &Path, ext: &str, dark: bool, images: std::collections::BTreeMap<String, Vec<u8>>, base: Option<&Path>, snapshots: &std::collections::BTreeMap<String, Vec<u8>>) -> anyhow::Result<()> {
    if ext == "sysml" {
        let doc = graphing_dsl::Document::parse(src);
        std::fs::write(path, graphing_export::to_sysml2(doc.diagram()))?;
        return Ok(());
    }
    let png = ext == "png";
    let scene = scene_of(src);
    let opts = SvgOptions { theme: if dark { Theme::dark() } else { Theme::light() }, images, icons: icons_for(&scene), refs: graphing_export::refs_for(&scene, base, snapshots), ..Default::default() };
    if png {
        std::fs::write(path, to_png(&scene, &opts, 2.0)?)?;
    } else {
        std::fs::write(path, to_svg(&scene, &opts))?;
    }
    Ok(())
}

/// Write the `animate` steps of `src`: GIF, WebM video, animated PNG
/// (`.png`/`.apng`) or animated SVG, by `path`'s extension.
pub fn write_animation(src: &str, path: &Path, dark: bool, images: std::collections::BTreeMap<String, Vec<u8>>, base: Option<&Path>, snapshots: &std::collections::BTreeMap<String, Vec<u8>>) -> anyhow::Result<()> {
    let (scene, timeline) = graphing_export::animation_of(src);
    let opts = SvgOptions { theme: if dark { Theme::dark() } else { Theme::light() }, images, icons: icons_for(&scene), refs: graphing_export::refs_for(&scene, base, snapshots), ..Default::default() };
    let anim = graphing_export::AnimOptions::default();
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let bytes = match ext.as_str() {
        "png" | "apng" => graphing_export::to_apng(&scene, &opts, &timeline, &anim)?,
        "svg" => graphing_export::to_svg_animated(&scene, &opts, &timeline).into_bytes(),
        "webm" => graphing_export::to_webm(&scene, &opts, &timeline, &anim)?,
        _ => graphing_export::to_gif(&scene, &opts, &timeline, &anim)?,
    };
    std::fs::write(path, bytes)?;
    Ok(())
}

/// The Lucide SVG files `scene`'s shapes draw, for SVG and PNG export.
pub fn icons_for(scene: &graphing_scene::Scene) -> std::collections::BTreeMap<String, String> {
    use gpui_kit::AssetSource as _;
    graphing_export::icons_used(scene)
        .into_iter()
        .filter_map(|name| {
            let bytes = gpui_kit::assets::AllAssets.load(&graphing_ui::kit::icon_path(&name)).ok()??;
            Some((name, String::from_utf8_lossy(&bytes).into_owned()))
        })
        .collect()
}

/// `.gph` source and warnings for a mermaid, draw.io, SysML v2 or Visio file.
/// `format` is the one the user picked, or `None` to tell by the file.
pub fn import(path: &Path, format: Option<graphing_import::Format>) -> anyhow::Result<(String, Vec<String>)> {
    let out = graphing_import::import_file_as(path, format)?;
    Ok((out.source, out.warnings))
}

#[cfg(test)]
mod tests {
    use gpui_kit::AssetSource as _;

    #[test]
    fn every_icon_a_pack_names_ships() {
        let reg = graphing_scene::stencils::registry();
        let mut names: Vec<(String, String)> = Vec::new();
        for s in reg.stencils() {
            names.extend(s.icon.iter().map(|i| (s.id.clone(), i.clone())));
            names.extend(s.glyph.iter().map(|g| (s.id.clone(), g.icon.clone())));
        }
        names.extend(reg.group_kinds.iter().filter_map(|k| k.icon.clone().map(|i| (k.name.clone(), i))));
        names.extend(reg.diagram_kinds.iter().filter_map(|k| k.icon.clone().map(|i| (k.id.clone(), i))));
        let missing: Vec<String> = names
            .into_iter()
            .filter(|(_, icon)| !matches!(gpui_kit::assets::AllAssets.load(&graphing_ui::kit::icon_path(icon)), Ok(Some(_))))
            .map(|(owner, icon)| format!("{owner}: {icon}"))
            .collect();
        assert!(missing.is_empty(), "icons that don't exist: {missing:?}");
    }

    #[test]
    fn exported_svg_carries_device_icons() {
        let scene = graphing_export::scene_of("r: net.router \"Edge\"\n");
        let icons = super::icons_for(&scene);
        assert!(icons.get("Router").is_some_and(|svg| svg.contains("<svg")));
    }
}
