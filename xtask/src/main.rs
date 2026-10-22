//! Repo tasks, run as `cargo xtask <task>`.
//!
//! `layers` enforces the dependency rules in docs/architecture.md as a build
//! failure rather than a convention. `credits` regenerates the open-source
//! list the About window shows. `capi` builds the C library and writes its
//! header off the Rust source. `ci` runs every gate.

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
    // the ABI reads moc3 and the Cubism JSON files through ayatsuri-import
    // with its default features off: no paint or project parsers in it
    (
        "crates/capi",
        &["openmoc3", "psd", "zip", "image", "quick-xml"],
        "the ABI takes only the moc3 and JSON side of the import crate",
    ),
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

/// Where the About window reads its open-source list from.
const CREDITS: &str = "assets/credits.tsv";
/// What ships: every crate these binaries pull in as a normal dependency,
/// on any platform (dev and build-only crates never reach a user).
const SHIPPED: &[&str] = &["ayatsuri", "ayatsuri-launcher"];

/// One line per third-party crate: name, version, licence, homepage.
fn credits_table() -> Result<String> {
    let out =
        Command::new("cargo").args(["metadata", "--format-version", "1", "--locked"]).output()?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned().into());
    }
    let meta: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    let empty = Vec::new();
    let packages = meta["packages"].as_array().unwrap_or(&empty);
    let nodes = meta["resolve"]["nodes"].as_array().unwrap_or(&empty);
    let members: std::collections::HashSet<&str> = meta["workspace_members"]
        .as_array()
        .unwrap_or(&empty)
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    let by_id: std::collections::HashMap<&str, &serde_json::Value> =
        packages.iter().filter_map(|p| Some((p["id"].as_str()?, p))).collect();
    let node_of: std::collections::HashMap<&str, &serde_json::Value> =
        nodes.iter().filter_map(|n| Some((n["id"].as_str()?, n))).collect();

    // walk the normal-dependency graph out from the shipped binaries
    let mut stack: Vec<&str> = packages
        .iter()
        .filter(|p| p["name"].as_str().is_some_and(|n| SHIPPED.contains(&n)))
        .filter_map(|p| p["id"].as_str())
        .collect();
    let mut seen = std::collections::HashSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(node) = node_of.get(id) else { continue };
        for dep in node["deps"].as_array().unwrap_or(&empty) {
            let normal =
                dep["dep_kinds"].as_array().unwrap_or(&empty).iter().any(|k| k["kind"].is_null());
            if normal && let Some(d) = dep["pkg"].as_str() {
                stack.push(d);
            }
        }
    }

    let mut rows: Vec<String> = seen
        .iter()
        .filter(|id| !members.contains(*id))
        .filter_map(|id| by_id.get(*id))
        .map(|p| {
            let field =
                |k: &str| p[k].as_str().unwrap_or("").replace(['\t', '\n'], " ").trim().to_string();
            let link = Some(field("repository"))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| field("homepage"));
            let licence = Some(field("license")).filter(|s| !s.is_empty()).unwrap_or_else(|| {
                if p["license_file"].is_string() {
                    "see licence file".into()
                } else {
                    "unknown".into()
                }
            });
            format!("{}\t{}\t{}\t{}", field("name"), field("version"), licence, link)
        })
        .collect();
    rows.sort_by_key(|r| r.to_lowercase());
    rows.dedup();
    let mut text = String::from(
        "# Generated by `cargo xtask credits`; do not edit by hand.\n\
         # name\tversion\tlicence\tlink\n",
    );
    for r in rows {
        text.push_str(&r);
        text.push('\n');
    }
    Ok(text)
}

fn credits(check: bool) -> Result<()> {
    let fresh = credits_table()?;
    let lines = fresh.lines().filter(|l| !l.starts_with('#')).count();
    if check {
        let current = std::fs::read_to_string(CREDITS).unwrap_or_default();
        if current != fresh {
            return Err(format!(
                "{CREDITS} is out of date with Cargo.lock; run `cargo xtask credits`"
            )
            .into());
        }
        println!("credits: ok ({lines} crates)");
    } else {
        std::fs::write(CREDITS, fresh)?;
        println!("credits: wrote {lines} crates to {CREDITS}");
    }
    Ok(())
}

/// `cargo xtask soak <model.aom> [seconds]`: run the headless soak example
/// in release, which fails when memory grows over the run.
fn soak(args: &[String]) -> Result<()> {
    let model = args.first().ok_or("usage: cargo xtask soak <model.aom> [seconds]")?;
    let mut argv =
        vec!["run", "--release", "-p", "ayatsuri", "--example", "soak", "--", model.as_str()];
    if let Some(secs) = args.get(1) {
        argv.push(secs.as_str());
    }
    run("cargo", &argv)
}

fn ci() -> Result<()> {
    run("cargo", &["fmt", "--check"])?;
    run("cargo", &["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"])?;
    run("cargo", &["test", "--workspace"])?;
    credits(true)?;
    capi(&["--check".to_string()])?;
    layers()
}

/// Where the C header lives in the repo, generated and committed so a host
/// needs no Rust toolchain to read it.
const HEADER: &str = "crates/capi/include/ayatsuri.h";

/// `cargo xtask capi`: build the C library under the stripped profile and
/// write `ayatsuri.h` from the crate's source, both into `target/capi/`.
/// `--header` only writes the header; `--check` fails when the committed
/// header is not what the source says.
fn capi(args: &[String]) -> Result<()> {
    let header_only = args.iter().any(|a| a == "--header");
    let check = args.iter().any(|a| a == "--check");
    let config = cbindgen::Config::from_file("crates/capi/cbindgen.toml")?;
    let bindings =
        cbindgen::Builder::new().with_crate("crates/capi").with_config(config).generate()?;
    let mut fresh = Vec::new();
    bindings.write(&mut fresh);
    let fresh = String::from_utf8(fresh)?;
    if check {
        let committed = std::fs::read_to_string(HEADER).unwrap_or_default();
        if committed != fresh {
            return Err(format!("{HEADER} is stale: run `cargo xtask capi`").into());
        }
        println!("{HEADER} matches the source");
        return Ok(());
    }
    std::fs::create_dir_all("crates/capi/include")?;
    std::fs::write(HEADER, &fresh)?;
    println!("wrote {HEADER}");
    if header_only {
        return Ok(());
    }
    run("cargo", &["build", "--profile", "capi-release", "-p", "ayatsuri-capi"])?;
    let out = Path::new("target/capi");
    std::fs::create_dir_all(out)?;
    std::fs::copy(HEADER, out.join("ayatsuri.h"))?;
    let mut shipped = vec!["ayatsuri.h".to_string()];
    for entry in std::fs::read_dir("target/capi-release")? {
        let path = entry?.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
        let ours = name.starts_with("libayatsuri_capi") || name.starts_with("ayatsuri_capi");
        let lib = [".so", ".a", ".dylib", ".dll", ".lib"].iter().any(|e| name.ends_with(e));
        if ours && lib {
            std::fs::copy(&path, out.join(&name))?;
            shipped.push(name);
        }
    }
    std::fs::copy("crates/capi/LICENSE", out.join("LICENSE"))?;
    println!("target/capi/: {}", shipped.join(", "));
    host_check(out)
}

/// The end-to-end check: the C host example compiled against the static
/// library and run on the fixture model. Skipped, with a note, when no C
/// compiler is on the path.
fn host_check(out: &Path) -> Result<()> {
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".into());
    if Command::new(&cc).arg("--version").output().is_err() {
        println!("no C compiler ({cc}) on the path; the host check is skipped");
        return Ok(());
    }
    let model = out.join("small.aom");
    let model_s = model.to_string_lossy().into_owned();
    run(
        "cargo",
        &[
            "run",
            "-q",
            "-p",
            "ayatsuri-capi",
            "--features",
            "fixture",
            "--example",
            "small_aom",
            "--",
            &model_s,
        ],
    )?;
    let exe = out.join(if cfg!(windows) { "host.exe" } else { "host" });
    let exe_s = exe.to_string_lossy().into_owned();
    // the archive by path, so the link is static and the linker's
    // gc-sections keeps only what the host's calls reach
    let archive = if cfg!(windows) {
        "target/capi-release/ayatsuri_capi.lib"
    } else {
        "target/capi-release/libayatsuri_capi.a"
    };
    let mut args =
        vec!["crates/capi/examples/host.c", "-Icrates/capi/include", "-o", &exe_s, archive];
    if cfg!(target_os = "linux") {
        args.extend(["-Wl,--gc-sections", "-lpthread", "-ldl", "-lm"]);
    } else if cfg!(target_os = "macos") {
        args.extend(["-Wl,-dead_strip", "-framework", "CoreFoundation"]);
    }
    run(&cc, &args)?;
    run(&exe_s, &[&model_s])?;
    if let Ok(meta) = std::fs::metadata(&exe) {
        println!("host linked statically: {} KB", meta.len() / 1024);
    }
    Ok(())
}

fn main() {
    let task = std::env::args().nth(1).unwrap_or_default();
    let result =
        match task.as_str() {
            "layers" => layers(),
            "credits" => credits(std::env::args().nth(2).is_some_and(|a| a == "--check")),
            "ci" => ci(),
            "soak" => soak(&std::env::args().skip(2).collect::<Vec<_>>()),
            "capi" => capi(&std::env::args().skip(2).collect::<Vec<_>>()),
            _ => Err("usage: cargo xtask <layers|credits [--check]|ci|capi [--header|--check]|soak <model.aom> [seconds]>"
                .into()),
        };
    if let Err(e) = result {
        eprintln!("xtask: {e}");
        std::process::exit(1);
    }
}
