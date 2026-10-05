//! File-level export and import used by the workspace commands.

use std::collections::BTreeMap;
use std::path::Path;

use graphing_export::{AnimOptions, SvgOptions, Theme};

/// How to render, besides the format the file name picks.
#[derive(Debug, Clone, Copy)]
pub struct Style {
    pub dark: bool,
    /// PNG pixels per diagram unit.
    pub scale: f32,
    /// `.svg` and `.png` play the `animate` steps instead of showing the
    /// diagram still. `.gif`, `.webm` and `.apng` always play them.
    pub animate: bool,
}

impl Default for Style {
    fn default() -> Self {
        Self { dark: false, scale: 2.0, animate: false }
    }
}

/// Write the diagram `src` to `out` in the format its extension names:
/// `.svg`, `.png`, `.gif`, `.webm`, `.apng` or `.sysml` (SysML v2 text).
/// `assets` are its packaged pictures and link snapshots, `base` the folder
/// of its own file, which pictures and links are relative to.
/// `progress`, when given, counts frames for an animation.
pub fn write(src: &str, assets: &BTreeMap<String, Vec<u8>>, base: Option<&Path>, out: &Path, style: Style, progress: Option<std::sync::Arc<graphing_export::Progress>>) -> anyhow::Result<()> {
    let ext = out.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let doc = graphing_dsl::Document::parse(src);
    if ext == "sysml" {
        std::fs::write(out, graphing_export::to_sysml2(doc.diagram()))?;
        return Ok(());
    }
    let (scene, timeline) = graphing_export::animation_of(src);
    let opts = SvgOptions {
        theme: if style.dark { Theme::dark() } else { Theme::light() },
        images: graphing_export::pictures(doc.diagram(), assets, base),
        icons: icons_for(&scene),
        refs: graphing_export::refs_for(&scene, base, &graphing_export::snapshots(assets)),
        ..Default::default()
    };
    let anim = AnimOptions { scale: style.scale.min(2.0), progress, ..Default::default() };
    let bytes = match (ext.as_str(), style.animate) {
        ("gif", _) => graphing_export::to_gif(&scene, &opts, &timeline, &anim)?,
        ("webm", _) => graphing_export::to_webm(&scene, &opts, &timeline, &anim)?,
        ("png", true) | ("apng", _) => graphing_export::to_apng(&scene, &opts, &timeline, &anim)?,
        ("png", false) => graphing_export::to_png(&scene, &opts, style.scale)?,
        (_, true) => graphing_export::to_svg_animated(&scene, &opts, &timeline).into_bytes(),
        (_, false) => graphing_export::to_svg(&scene, &opts).into_bytes(),
    };
    std::fs::write(out, bytes)?;
    Ok(())
}

/// The Lucide SVG files `scene`'s shapes draw, for SVG and PNG export.
pub fn icons_for(scene: &graphing_scene::Scene) -> BTreeMap<String, String> {
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
