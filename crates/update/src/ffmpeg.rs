//! An ffmpeg for video export (MP4, WebM, MOV), so nobody has to install one
//! and put it on PATH. Nothing ships with Ayatsuri2D: on request, a public
//! static build is downloaded straight from the people who build it and
//! kept in a folder of our own. No admin rights, no PATH changes; deleting
//! the folder removes it.
//!
//! ```text
//!   Windows x64        BtbN/FFmpeg-Builds on GitHub, the newest release
//!                      branch, GPL build   (checksums.sha256 beside it)
//!   Linux, macOS       ffmpeg.martin-riedl.de, the newest release
//!                      (Apple Silicon: the newest snapshot, the only one
//!                      built for it)       (<file>.sha256 beside it)
//! ```
//!
//! Where it goes: `ffmpeg/` beside the editor when an installer fetched it
//! (`ayatsuri-launcher --get-ffmpeg <dir>`), otherwise `ffmpeg/` in the
//! preferences folder. Which one runs: ffmpeg on PATH first (the user's own
//! choice), then beside the editor, then the preferences folder.
//!
//! As with updates, a download that does not hash to its published line is
//! never written; that proves the bytes match the checksum, and HTTPS to the
//! builder stands behind the checksum.

use std::io::{Read, Seek};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::install::{digest_for, verify};

/// The program's file name on this system.
pub const EXE: &str = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };

const BTBN_LATEST: &str = "https://api.github.com/repos/BtbN/FFmpeg-Builds/releases/tags/latest";
const RIEDL: &str = "https://ffmpeg.martin-riedl.de";
/// The Windows build is about 190 MB; nothing real is bigger.
const MAX_ARCHIVE: u64 = 400 * 1024 * 1024;

/// The folders our own copy can be in, in the order they are tried.
pub fn our_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf))
    {
        dirs.push(dir.join("ffmpeg"));
    }
    if let Some(dir) = crate::config_dir() {
        dirs.push(dir.join("ffmpeg"));
    }
    dirs
}

/// Where a download from inside the editor goes.
pub fn user_dir() -> Option<PathBuf> {
    crate::config_dir().map(|d| d.join("ffmpeg"))
}

fn runs(program: &Path) -> bool {
    let mut cmd = std::process::Command::new(program);
    cmd.arg("-version").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // no console window flashing up behind the editor
        cmd.creation_flags(0x0800_0000);
    }
    cmd.status().is_ok_and(|s| s.success())
}

/// The ffmpeg to run, if there is one that works. Starts a process or two,
/// so call it when a window opens, not every frame.
///
/// AYATSURI_PRETEND_NO_FFMPEG=1 ignores the one on PATH, to try the offer
/// and the download on a machine that has ffmpeg.
pub fn program() -> Option<PathBuf> {
    let on_path = PathBuf::from(EXE);
    if std::env::var_os("AYATSURI_PRETEND_NO_FFMPEG").is_none() && runs(&on_path) {
        return Some(on_path);
    }
    our_dirs().into_iter().map(|d| d.join(EXE)).find(|p| p.is_file() && runs(p))
}

/// Whether there is a build to download for this system.
pub fn can_fetch() -> bool {
    archive_for().is_some()
}

enum Archive {
    /// The newest release-branch GPL zip from BtbN's `latest` release.
    Btbn,
    /// martin-riedl.de's redirect for this system: `<os>/<arch>/<channel>`.
    Riedl(&'static str),
}

fn archive_for() -> Option<Archive> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Some(Archive::Btbn),
        ("linux", "x86_64") => Some(Archive::Riedl("linux/amd64/release")),
        ("linux", "aarch64") => Some(Archive::Riedl("linux/arm64/release")),
        ("macos", "x86_64") => Some(Archive::Riedl("macos/amd64/release")),
        ("macos", "aarch64") => Some(Archive::Riedl("macos/arm64/snapshot")),
        _ => None,
    }
}

fn agent(redirects: u32) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .max_redirects(redirects)
        .http_status_as_error(redirects > 0)
        .build()
        .into()
}

fn user_agent() -> String {
    format!("Ayatsuri2D/{}", env!("CARGO_PKG_VERSION"))
}

fn get_text(url: &str) -> Result<String, String> {
    agent(5)
        .get(url)
        .header("User-Agent", &user_agent())
        .call()
        .map_err(|e| format!("could not reach {url}: {e}"))?
        .body_mut()
        .with_config()
        .limit(1024 * 1024)
        .read_to_string()
        .map_err(|e| format!("could not read {url}: {e}"))
}

/// The archive's URL, its file name in the checksum list, and that list.
fn locate(archive: &Archive) -> Result<(String, String, String), String> {
    match archive {
        Archive::Btbn => {
            #[derive(serde::Deserialize)]
            struct Release {
                assets: Vec<crate::install::Asset>,
            }
            let rel: Release = serde_json::from_str(&get_text(BTBN_LATEST)?)
                .map_err(|_| "GitHub's answer about the ffmpeg builds was not readable")?;
            let sums = rel
                .assets
                .iter()
                .find(|a| a.name == "checksums.sha256")
                .ok_or("the ffmpeg builds have no checksums, so nothing could be checked")?;
            let zip = newest_btbn(rel.assets.iter().map(|a| a.name.as_str()))
                .and_then(|n| rel.assets.iter().find(|a| a.name == n))
                .ok_or("there is no Windows ffmpeg build right now")?;
            Ok((
                zip.browser_download_url.clone(),
                zip.name.clone(),
                get_text(&sums.browser_download_url)?,
            ))
        }
        Archive::Riedl(which) => {
            // the redirect names the build; its checksum sits beside it
            let probe = format!("{RIEDL}/redirect/latest/{which}/ffmpeg.zip");
            let resp = agent(0)
                .get(&probe)
                .header("User-Agent", &user_agent())
                .call()
                .map_err(|e| format!("could not reach {probe}: {e}"))?;
            let to = resp
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .ok_or("ffmpeg.martin-riedl.de did not name a build for this system")?;
            let url =
                if to.starts_with("https://") { to.to_string() } else { format!("{RIEDL}{to}") };
            let sums = get_text(&format!("{url}.sha256"))?;
            Ok((url, "ffmpeg.zip".into(), sums))
        }
    }
}

/// From BtbN's file names, the GPL Windows zip of the highest release
/// branch (`ffmpeg-n9.0-latest-win64-gpl-9.0.zip`), never master or shared.
fn newest_btbn<'a>(names: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    names
        .filter_map(|n| {
            let ver = n.strip_prefix("ffmpeg-n")?.strip_suffix(".zip")?;
            let (branch, rest) = ver.split_once("-latest-win64-gpl-")?;
            (branch == rest).then_some(())?;
            let mut parts = branch.split('.').map(|p| p.parse::<u32>().ok());
            let key = (parts.next()??, parts.next().flatten().unwrap_or(0));
            Some((key, n))
        })
        .max_by_key(|(k, _)| *k)
        .map(|(_, n)| n)
}

/// Download, check and unpack ffmpeg into `dir`, and return the program.
/// `progress` counts the bytes for a bar. Blocking, for minutes on a slow line: run it off the UI
/// thread.
pub fn fetch(dir: &Path, progress: &crate::Progress) -> Result<PathBuf, String> {
    let archive = archive_for().ok_or("there is no ffmpeg download for this system")?;
    let (url, name, sums) = locate(&archive)?;
    let want = digest_for(&sums, &name)
        .ok_or_else(|| format!("the checksum list has no line for {name}"))?;
    log::info!("ffmpeg: downloading {url}");
    let mut resp = agent(5)
        .get(&url)
        .header("User-Agent", &user_agent())
        .call()
        .map_err(|e| format!("could not download {url}: {e}"))?;
    let bytes = crate::read_counted(&mut resp, MAX_ARCHIVE, progress)?;
    verify(&bytes, &want)?;
    let program = entry_in_zip(std::io::Cursor::new(bytes), EXE)?;
    std::fs::create_dir_all(dir).map_err(|e| format!("could not make {}: {e}", dir.display()))?;
    let target = dir.join(EXE);
    let part = dir.join(format!("{EXE}.part"));
    std::fs::write(&part, &program).map_err(|e| format!("could not save ffmpeg: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("could not make ffmpeg runnable: {e}"))?;
    }
    std::fs::rename(&part, &target).map_err(|e| format!("could not put ffmpeg in place: {e}"))?;
    if !runs(&target) {
        let _ = std::fs::remove_file(&target);
        return Err("the downloaded ffmpeg does not run on this system".into());
    }
    log::info!("ffmpeg: ready at {}", target.display());
    Ok(target)
}

/// The one file in the zip called `name` (in any folder). Names are only
/// compared, never used as paths, so `../` in one goes nowhere.
fn entry_in_zip(reader: impl Read + Seek, name: &str) -> Result<Vec<u8>, String> {
    let mut zip = zip::ZipArchive::new(reader).map_err(|_| "the download is not a readable zip")?;
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).map_err(|e| format!("the zip is damaged: {e}"))?;
        let path = file.name().map_err(|e| format!("the zip is damaged: {e}"))?.replace('\\', "/");
        if path.rsplit('/').next() == Some(name) && file.is_file() {
            let mut out = Vec::with_capacity(file.size() as usize);
            file.read_to_end(&mut out).map_err(|e| format!("the zip is damaged: {e}"))?;
            return Ok(out);
        }
    }
    Err(format!("the download has no {name} in it"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_highest_release_branch_gpl_zip() {
        let names = [
            "checksums.sha256",
            "ffmpeg-master-latest-win64-gpl.zip",
            "ffmpeg-n8.1-latest-win64-gpl-8.1.zip",
            "ffmpeg-n9.0-latest-win64-lgpl-9.0.zip",
            "ffmpeg-n9.0-latest-win64-gpl-9.0.zip",
            "ffmpeg-n9.0-latest-win64-gpl-shared-9.0.zip",
            "ffmpeg-n10.0-latest-linux64-gpl-10.0.tar.xz",
        ];
        assert_eq!(
            newest_btbn(names.iter().copied()),
            Some("ffmpeg-n9.0-latest-win64-gpl-9.0.zip")
        );
        assert_eq!(newest_btbn(["ffmpeg-master-latest-win64-gpl.zip"].into_iter()), None);
    }

    /// Asks the network: `cargo test -p ayatsuri-update -- --ignored`.
    #[test]
    #[ignore]
    fn every_source_names_a_build_and_its_checksum() {
        for archive in [
            Archive::Btbn,
            Archive::Riedl("linux/amd64/release"),
            Archive::Riedl("linux/arm64/release"),
            Archive::Riedl("macos/amd64/release"),
            Archive::Riedl("macos/arm64/snapshot"),
        ] {
            let (url, name, sums) = locate(&archive).unwrap();
            assert!(digest_for(&sums, &name).is_some(), "{url}: no checksum for {name}");
        }
    }

    #[test]
    fn finds_the_program_in_any_folder_of_the_zip() {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            let opt = zip::write::SimpleFileOptions::default();
            w.start_file("ffmpeg-n9.0/doc/ffmpeg.html", opt).unwrap();
            std::io::Write::write_all(&mut w, b"doc").unwrap();
            w.start_file("ffmpeg-n9.0/bin/ffmpeg.exe", opt).unwrap();
            std::io::Write::write_all(&mut w, b"MZ").unwrap();
            w.finish().unwrap();
        }
        buf.set_position(0);
        assert_eq!(entry_in_zip(buf.clone(), "ffmpeg.exe").unwrap(), b"MZ");
        assert!(entry_in_zip(buf, "ffprobe.exe").is_err());
    }
}
