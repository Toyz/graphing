use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "graphing", version, about = "Visual graph and diagram builder")]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
    /// Extra shape pack folders to load (in addition to the config folder).
    #[arg(long = "packs", global = true)]
    packs: Vec<PathBuf>,
    /// Diagrams to open as tabs. With none, the last session is restored.
    files: Vec<PathBuf>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Parse a file and print warnings; exits non-zero if there are any.
    Check { file: PathBuf },
    /// Render a diagram to SVG, PNG, GIF, WebM or SysML v2 text (by output
    /// extension). `.gif` and `.webm` play the diagram's `animate` steps.
    Render {
        file: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
        /// Dark theme colors.
        #[arg(long)]
        dark: bool,
        /// PNG pixels per diagram unit.
        #[arg(long, default_value_t = 2.0)]
        scale: f32,
        /// Play the `animate` steps: `.svg` becomes an animated SVG, `.png`
        /// an animated PNG.
        #[arg(long)]
        animate: bool,
    },
    /// Convert a mermaid, draw.io, SysML v2 or Visio (.vsdx) file to .gph
    /// (stdout without -o).
    Import {
        file: PathBuf,
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Make a .gphz package from a .gph (linked pictures go inside) or from a
    /// folder made by `unpack`.
    Pack {
        input: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Expand a .gphz into a folder: diagram.gph plus assets/.
    Unpack {
        file: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Show what a .gphz holds and how much each part takes.
    Info { file: PathBuf },
}

fn main() -> ExitCode {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    let cli = Cli::parse();
    if cli.cmd.is_some() {
        for e in graphing_app::load_packs(&cli.packs) {
            eprintln!("warning: {e}");
        }
    }
    let result = match cli.cmd {
        Some(Cmd::Check { file }) => return check(&file),
        Some(Cmd::Render { file, out, dark, scale, animate }) => render(&file, &out, dark, scale, animate),
        Some(Cmd::Import { file, out }) => import(&file, out.as_deref()),
        Some(Cmd::Pack { input, out }) => pack(&input, &out),
        Some(Cmd::Unpack { file, out }) => unpack(&file, &out),
        Some(Cmd::Info { file }) => info(&file),
        None => graphing_app::run(cli.files),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("graphing: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn check(file: &Path) -> ExitCode {
    let src = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("graphing: {}: {e}", file.display());
            return ExitCode::FAILURE;
        }
    };
    let doc = graphing_dsl::Document::parse(src.as_str());
    for d in doc.diags() {
        let line = src[..d.span.start].matches('\n').count() + 1;
        let col = d.span.start - src[..d.span.start].rfind('\n').map_or(0, |i| i + 1) + 1;
        println!("{}:{line}:{col}: {}", file.display(), d.message);
    }
    let m = doc.diagram();
    println!("{} nodes, {} edges, {} groups", m.nodes.len(), m.edges.len(), m.groups.len());
    if doc.diags().is_empty() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

/// The diagram text, plus picture bytes by `src` (packaged assets and files
/// linked relative to the diagram).
/// Diagram text and its pictures (by `src`); see [`load_all`] for links.
fn load(file: &Path) -> anyhow::Result<(String, std::collections::BTreeMap<String, Vec<u8>>)> {
    load_all(file).map(|(src, images, _)| (src, images))
}

/// Text, pictures by `src`, and snapshots of linked diagrams by `src`.
#[allow(clippy::type_complexity)]
fn load_all(file: &Path) -> anyhow::Result<(String, std::collections::BTreeMap<String, Vec<u8>>, std::collections::BTreeMap<String, Vec<u8>>)> {
    let bytes = std::fs::read(file)?;
    let (src, assets) = if graphing_package::is_package(&bytes) {
        let pkg = graphing_package::read(&bytes)?;
        (pkg.doc, pkg.assets)
    } else {
        (String::from_utf8(bytes)?, Default::default())
    };
    let doc = graphing_dsl::Document::parse(src.as_str());
    let dir = file.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut images = std::collections::BTreeMap::new();
    // Pictures only: a link's `src` names a diagram.
    for n in doc.diagram().nodes.iter().filter(|n| !graphing_scene::notation::is_link(n)) {
        let Some(s) = doc.diagram().node_prop(n, "src").map(graphing_model::Value::text) else { continue };
        let bytes = match s.strip_prefix(graphing_package::ASSET_PREFIX) {
            Some(name) => assets.get(name).cloned(),
            None => std::fs::read(dir.join(&s)).ok(),
        };
        if let Some(b) = bytes {
            images.insert(s, b);
        }
    }
    let snapshots = graphing_export::snapshots(&assets);
    Ok((src, images, snapshots))
}

fn pack(input: &Path, out: &Path) -> anyhow::Result<()> {
    let mut pkg = graphing_package::Package::default();
    if input.is_dir() {
        // A folder from `unpack`: the text as is, every file under assets/.
        pkg.doc = std::fs::read_to_string(input.join("diagram.gph"))?;
        if let Ok(dir) = std::fs::read_dir(input.join("assets")) {
            for e in dir.flatten().filter(|e| e.path().is_file()) {
                pkg.assets.insert(e.file_name().to_string_lossy().to_string(), std::fs::read(e.path())?);
            }
        }
        // Snapshots of linked diagrams, as `unpack` left them.
        if let Ok(dir) = std::fs::read_dir(input.join("assets").join("linked")) {
            for e in dir.flatten().filter(|e| e.path().is_file()) {
                pkg.assets.insert(format!("{}{}", graphing_package::LINKED_PREFIX, e.file_name().to_string_lossy()), std::fs::read(e.path())?);
            }
        }
    } else {
        // A .gph: linked pictures move inside and their `src` is rewritten.
        let (mut text, images) = load(input)?;
        for (src, bytes) in images {
            if src.starts_with(graphing_package::ASSET_PREFIX) {
                continue;
            }
            let reference = pkg.add_asset(&src, bytes);
            text = text.replace(&format!("\"{src}\""), &format!("\"{reference}\""));
        }
        // Linked diagrams travel along as snapshots.
        graphing_export::snapshot_links(&text, input.parent(), &mut pkg.assets);
        pkg.doc = text;
    }
    std::fs::write(out, graphing_package::write(&pkg))?;
    let links = pkg.assets.keys().filter(|k| k.starts_with(graphing_package::LINKED_PREFIX)).count();
    println!("{} ({} bytes, {} picture(s), {links} linked diagram(s))", out.display(), std::fs::metadata(out)?.len(), pkg.assets.len() - links);
    Ok(())
}

fn unpack(file: &Path, out: &Path) -> anyhow::Result<()> {
    let pkg = graphing_package::read(&std::fs::read(file)?)?;
    std::fs::create_dir_all(out.join("assets"))?;
    std::fs::write(out.join("diagram.gph"), &pkg.doc)?;
    for (name, bytes) in &pkg.assets {
        // Linked diagram snapshots land in assets/linked/.
        let path = out.join("assets").join(name);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, bytes)?;
    }
    if !pkg.meta.is_empty() {
        std::fs::write(out.join("meta.json"), &pkg.meta)?;
    }
    println!("{} (diagram.gph, {} asset(s))", out.display(), pkg.assets.len());
    Ok(())
}

fn info(file: &Path) -> anyhow::Result<()> {
    let bytes = std::fs::read(file)?;
    let pkg = graphing_package::read(&bytes)?;
    println!("{}: {} bytes on disk", file.display(), bytes.len());
    println!("  diagram.gph  {} bytes of text", pkg.doc.len());
    for (name, b) in &pkg.assets {
        println!("  {name}  {} bytes", b.len());
    }
    let raw: usize = pkg.doc.len() + pkg.assets.values().map(Vec::len).sum::<usize>();
    println!("  {} bytes unpacked, {:.0}% of that on disk", raw, bytes.len() as f64 * 100.0 / raw.max(1) as f64);
    Ok(())
}

fn render(file: &Path, out: &Path, dark: bool, scale: f32, animate: bool) -> anyhow::Result<()> {
    use graphing_export::{AnimOptions, SvgOptions, Theme};
    let (src, images, snapshots) = load_all(file)?;
    let ext = out.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    // `.sysml` writes SysML v2 text instead of a picture.
    if ext == "sysml" {
        let doc = graphing_dsl::Document::parse(src);
        std::fs::write(out, graphing_export::to_sysml2(doc.diagram()))?;
        return Ok(());
    }
    let (scene, timeline) = graphing_export::animation_of(&src);
    let opts = SvgOptions { theme: if dark { Theme::dark() } else { Theme::light() }, images, icons: graphing_app::icons_for(&scene), refs: graphing_export::refs_for(&scene, file.parent(), &snapshots), ..Default::default() };
    let anim = AnimOptions { scale: scale.min(2.0), ..Default::default() };
    let bytes = match (ext.as_str(), animate) {
        ("gif", _) => graphing_export::to_gif(&scene, &opts, &timeline, &anim)?,
        ("webm", _) => graphing_export::to_webm(&scene, &opts, &timeline, &anim)?,
        ("png" | "apng", true) | ("apng", false) => graphing_export::to_apng(&scene, &opts, &timeline, &anim)?,
        ("png", false) => graphing_export::to_png(&scene, &opts, scale)?,
        (_, true) => graphing_export::to_svg_animated(&scene, &opts, &timeline).into_bytes(),
        (_, false) => graphing_export::to_svg(&scene, &opts).into_bytes(),
    };
    std::fs::write(out, bytes)?;
    Ok(())
}

fn import(file: &Path, out: Option<&Path>) -> anyhow::Result<()> {
    let imported = graphing_import::import_file(file)?;
    for w in &imported.warnings {
        eprintln!("warning: {w}");
    }
    match out {
        Some(p) => std::fs::write(p, imported.source)?,
        None => print!("{}", imported.source),
    }
    Ok(())
}
