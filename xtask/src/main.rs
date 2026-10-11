//! Repo tasks, run as `cargo xtask <task>`.
//!
//! `layers` enforces the dependency rules in docs/architecture.md as a build
//! failure rather than a convention. `ci` runs every gate.

use std::error::Error;
use std::path::Path;
use std::process::Command;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// (crate dir, forbidden dependencies, why)
const RULES: &[(&str, &[&str], &str)] = &[
    // The hostile-input surface stays isolated; the runtime never compiles it.
    (
        "crates/core",
        &["openmoc3", "ayatsuri-import"],
        "ayatsuri-core must never depend on openmoc3",
    ),
    ("crates/capi", &["openmoc3", "ayatsuri-import"], "the ABI never compiles the moc3 parser"),
    // Nothing below the editor knows the UI toolkit exists.
    ("crates/format", &["egui", "eframe"], "format is UI-free"),
    ("crates/core", &["egui", "eframe"], "core is UI-free"),
    ("crates/render", &["egui", "eframe"], "render is UI-free; backends plug in underneath"),
    ("crates/capi", &["egui", "eframe"], "the ABI is UI-free"),
    ("crates/import", &["egui", "eframe"], "import is UI-free"),
    ("crates/openmoc3", &["egui", "eframe"], "openmoc3 is a standalone parser"),
];

fn deps_of(manifest: &Path) -> Result<Vec<String>> {
    let text = std::fs::read_to_string(manifest)?;
    let value: toml::Table = text.parse()?;
    let mut out = Vec::new();
    for table in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(deps) = value.get(table).and_then(|d| d.as_table()) {
            out.extend(deps.keys().cloned());
        }
    }
    Ok(out)
}

fn layers() -> Result<()> {
    let mut failures = 0;
    for (dir, forbidden, why) in RULES {
        let manifest = Path::new(dir).join("Cargo.toml");
        if !manifest.exists() {
            continue;
        }
        let deps = deps_of(&manifest)?;
        for f in *forbidden {
            if deps.iter().any(|d| d == f) {
                eprintln!("layers: {dir} depends on {f}: {why}");
                failures += 1;
            }
        }
    }
    if failures > 0 {
        return Err(format!("{failures} layering violation(s)").into());
    }
    println!("layers: ok");
    Ok(())
}

fn run(program: &str, args: &[&str]) -> Result<()> {
    println!("$ {program} {}", args.join(" "));
    let status = Command::new(program).args(args).status()?;
    if !status.success() {
        return Err(format!("{program} {} failed", args.join(" ")).into());
    }
    Ok(())
}

fn ci() -> Result<()> {
    run("cargo", &["fmt", "--check"])?;
    run("cargo", &["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"])?;
    run("cargo", &["test", "--workspace"])?;
    layers()
}

fn main() {
    let task = std::env::args().nth(1).unwrap_or_default();
    let result = match task.as_str() {
        "layers" => layers(),
        "ci" => ci(),
        _ => Err("usage: cargo xtask <layers|ci>".into()),
    };
    if let Err(e) = result {
        eprintln!("xtask: {e}");
        std::process::exit(1);
    }
}
