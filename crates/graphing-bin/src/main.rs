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
    let src = match load(file) {
        Ok(p) => p.doc,
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

/// A `.gph` or `.gphz` file as a package (plain text has no assets).
fn load(file: &Path) -> anyhow::Result<graphing_package::Package> {
    Ok(graphing_package::open(std::fs::read(file)?)?)
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
                pkg.assets.insert(graphing_package::linked_name(&e.file_name().to_string_lossy()), std::fs::read(e.path())?);
            }
        }
    } else {
        // A .gph: linked pictures move inside and their `src` is rewritten.
        let mut text = load(input)?.doc;
        let doc = graphing_dsl::Document::parse(text.as_str());
        for (src, bytes) in graphing_export::pictures(doc.diagram(), &Default::default(), input.parent()) {
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
    let pkg = load(file)?;
    graphing_app::export::write(&pkg.doc, &pkg.assets, file.parent(), out, graphing_app::export::Style { dark, scale, animate })
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
