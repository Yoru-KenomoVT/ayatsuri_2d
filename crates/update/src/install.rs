//! Replacing this copy with a newer release, where that is this copy's to do.
//!
//! Who owns the files decides it, never a guess:
//!
//! ```text
//!   an AppImage (APPIMAGE set)          replace that one file
//!   /usr, /opt, /snap, /nix, /app       a package's; point at the page
//!   inside a .app bundle                the .dmg is the unit; the page
//!   Program Files / ProgramData         run the new setup.exe /S on exit
//!   any other Windows folder            the portable zip; the page
//!   anywhere else on Linux              ours: swap in the tarball's binaries
//!   could not be worked out             nothing on a guess; the page
//! ```
//!
//! File names come from the release's own asset list and are vouched for by
//! its SHA256SUMS.txt; a download that does not hash to its line is never
//! written or run. That proves the bytes are the bytes the manifest names,
//! not that the manifest is genuine: only HTTPS to GitHub stands behind that.

use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// How this copy was installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Install {
    /// A Linux tree we put there (install.sh into ~/.local, or an unpacked
    /// tarball): the binaries in `dir` are ours to replace.
    Tarball { dir: PathBuf },
    /// An AppImage at this path.
    AppImage { file: PathBuf },
    /// Installed by the Windows installer: the new installer updates it.
    WindowsInstaller,
    /// Not this copy's to rewrite; the reason is shown to the user.
    Elsewhere { why: &'static str },
}

impl Install {
    /// Whether "Update now" can do the whole job.
    pub fn can_update(&self) -> bool {
        !matches!(self, Install::Elsewhere { .. })
    }
}

/// The file a running process was started from, even after it was replaced
/// underneath (Linux reports that path with " (deleted)" on the end).
fn running_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    if exe.exists() {
        return Some(exe);
    }
    let text = exe.to_string_lossy().to_string();
    let candidate = PathBuf::from(text.strip_suffix(" (deleted)")?);
    candidate.exists().then_some(candidate)
}

const SYSTEM_PREFIXES: [&str; 7] =
    ["/usr/", "/opt/", "/bin/", "/sbin/", "/snap/", "/nix/store/", "/app/"];

/// Work out how this copy was installed.
pub fn detect() -> Install {
    if let Some(file) = std::env::var_os("APPIMAGE").map(PathBuf::from)
        && file.is_file()
    {
        let text = file.to_string_lossy().replace('\\', "/");
        if SYSTEM_PREFIXES.iter().any(|p| text.starts_with(p)) {
            return Install::Elsewhere { why: "this AppImage sits in a system folder" };
        }
        return Install::AppImage { file };
    }
    let Some(exe) = running_exe() else {
        return Install::Elsewhere { why: "this program's own file could not be found" };
    };
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    let text = exe.to_string_lossy().replace('\\', "/");
    if text.contains(".app/Contents/MacOS/") {
        return Install::Elsewhere { why: "on macOS the new version comes as a disk image" };
    }
    if cfg!(windows) {
        let roots: Vec<String> =
            ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432", "ProgramData"]
                .iter()
                .filter_map(|n| std::env::var(n).ok())
                .collect();
        return if in_any(&text, &roots) {
            Install::WindowsInstaller
        } else {
            Install::Elsewhere { why: "this is the portable copy: unzip the new one over it" }
        };
    }
    if !cfg!(target_os = "linux") {
        return Install::Elsewhere { why: "there is no self-update for this system yet" };
    }
    if SYSTEM_PREFIXES.iter().any(|p| text.starts_with(p)) {
        return Install::Elsewhere {
            why: "a package manager owns this copy: install the new .deb or .rpm",
        };
    }
    match exe.parent() {
        Some(dir) => Install::Tarball { dir: dir.to_path_buf() },
        None => Install::Elsewhere { why: "this program's folder could not be found" },
    }
}

/// Whether `path` lies under one of `roots`, case-insensitively, with a
/// separator after the root (C:/ProgramData is not C:/ProgramDataX).
fn in_any(path: &str, roots: &[String]) -> bool {
    let path = path.replace('\\', "/").to_ascii_lowercase();
    roots.iter().any(|root| {
        let root = root.replace('\\', "/").to_ascii_lowercase();
        let root = root.trim_end_matches('/');
        !root.is_empty() && path.starts_with(&format!("{root}/"))
    })
}

/// The release file this kind of install updates from: how its name ends.
fn wanted_suffix(install: &Install) -> Option<String> {
    let arch = std::env::consts::ARCH;
    match install {
        Install::Tarball { .. } => Some(format!("-linux-{arch}.tar.gz")),
        Install::AppImage { .. } => Some(format!("-linux-{arch}.AppImage")),
        Install::WindowsInstaller => Some("-windows-x64-setup.exe".to_string()),
        Install::Elsewhere { .. } => None,
    }
}

/// One file attached to a release.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
}

/// What an update did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Installed {
    /// The new binaries are in place; the old ones wait beside them as
    /// `<name>.old` until the next start. Restart to run the new version.
    Replaced,
    /// The installer is downloaded and checked; run it on exit.
    Installer { path: PathBuf, sha256: String },
}

const MAX_ARTEFACT: u64 = 256 * 1024 * 1024;
const MAX_MANIFEST: u64 = 256 * 1024;

fn get(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    ureq::get(url)
        .header("User-Agent", &format!("Ayatsuri2D/{}", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|e| format!("could not download {url}: {e}"))?
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()
        .map_err(|e| format!("the download from {url} did not finish: {e}"))
}

/// The digest SHA256SUMS gives a file, from `sha256sum`'s own format.
pub fn digest_for(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let digest = parts.next()?;
        let file = parts.next()?.trim_start_matches('*');
        (file == name && digest.len() == 64).then(|| digest.to_ascii_lowercase())
    })
}

/// The bytes' sha256 against the published one.
pub fn verify(bytes: &[u8], want: &str) -> Result<(), String> {
    let got: String = Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect();
    if got.eq_ignore_ascii_case(want) {
        Ok(())
    } else {
        Err("the download does not match its published checksum, so it was not used".into())
    }
}

/// Fetch, check and put in place the update for this kind of install, from
/// a release's assets. Blocking; run it off the UI thread.
pub fn install(install: &Install, assets: &[Asset]) -> Result<Installed, String> {
    install_counted(install, assets, &crate::Progress::default())
}

/// `install`, counting the release's download into `progress` for a bar.
pub fn install_counted(
    install: &Install,
    assets: &[Asset],
    progress: &crate::Progress,
) -> Result<Installed, String> {
    let suffix = wanted_suffix(install).ok_or("this copy does not update itself")?;
    let sums = assets
        .iter()
        .find(|a| a.name == "SHA256SUMS.txt")
        .ok_or("the release has no SHA256SUMS.txt, so nothing could be checked")?;
    let artefact = assets
        .iter()
        .find(|a| a.name.ends_with(&suffix))
        .ok_or("the release has no build for this system yet")?;
    let sums = String::from_utf8(get(&sums.browser_download_url, MAX_MANIFEST)?)
        .map_err(|_| "SHA256SUMS.txt is not text")?;
    let want = digest_for(&sums, &artefact.name)
        .ok_or_else(|| format!("SHA256SUMS.txt has no line for {}", artefact.name))?;
    let url = &artefact.browser_download_url;
    let mut resp = ureq::get(url)
        .header("User-Agent", &format!("Ayatsuri2D/{}", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|e| format!("could not download {url}: {e}"))?;
    let bytes = crate::read_counted(&mut resp, MAX_ARTEFACT, progress)?;
    verify(&bytes, &want)?;

    match install {
        Install::Tarball { dir } => {
            let tar = gunzip(&bytes)?;
            let mut placed = 0;
            for name in ["ayatsuri", "ayatsuri-launcher"] {
                let target = dir.join(name);
                if !target.exists() {
                    continue;
                }
                let binary = entry_in_tar(&tar, &format!("/bin/{name}"))
                    .ok_or_else(|| format!("{} has no bin/{name}", artefact.name))?;
                stage_then_rename(&target, &binary)?;
                placed += 1;
            }
            if placed == 0 {
                return Err("no Ayatsuri2D program was found to replace".into());
            }
            Ok(Installed::Replaced)
        }
        Install::AppImage { file } => stage_then_rename(file, &bytes).map(|()| Installed::Replaced),
        Install::WindowsInstaller => {
            let path = std::env::temp_dir().join(&artefact.name);
            std::fs::write(&path, &bytes)
                .map_err(|e| format!("the installer could not be saved: {e}"))?;
            Ok(Installed::Installer { path, sha256: want })
        }
        Install::Elsewhere { why } => Err((*why).to_string()),
    }
}

fn gunzip(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_end(&mut out)
        .map_err(|_| "the download is not a readable archive".to_string())?;
    Ok(out)
}

/// One file out of a tar archive, by how its name ends. Names in the archive
/// are only compared, never used as paths, so `../` in one goes nowhere.
fn entry_in_tar(archive: &[u8], ends_with: &str) -> Option<Vec<u8>> {
    let mut at = 0usize;
    loop {
        let header = archive.get(at..at.checked_add(512)?)?;
        if header.iter().all(|b| *b == 0) {
            return None;
        }
        let name = text_field(header.get(0..100)?);
        let size = usize::from_str_radix(text_field(header.get(124..136)?).trim(), 8).unwrap_or(0);
        let kind = header.get(156).copied().unwrap_or(0);
        let start = at.checked_add(512)?;
        let end = start.checked_add(size)?;
        if (kind == b'0' || kind == 0) && name.ends_with(ends_with) {
            return archive.get(start..end).map(<[u8]>::to_vec);
        }
        at = start.checked_add(size.div_ceil(512).checked_mul(512)?)?;
    }
}

fn text_field(field: &[u8]) -> String {
    let end = field.iter().position(|b| *b == 0).unwrap_or(field.len());
    String::from_utf8_lossy(field.get(..end).unwrap_or(&[])).trim().to_string()
}

/// Where an update leaves the previous version: `<name>.old` beside it.
fn previous_beside(path: &Path) -> Option<PathBuf> {
    let mut name = path.file_name()?.to_os_string();
    name.push(".old");
    Some(path.parent()?.join(name))
}

/// Write the new file beside the old one and rename it over the top: the
/// name never holds a half-written file, and the old one is kept as .old.
fn stage_then_rename(target: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = target.parent().ok_or("no folder to update in")?;
    let name = target.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let staged = dir.join(format!(".{name}.new"));
    let kept = previous_beside(target).ok_or("no folder to update in")?;
    std::fs::write(&staged, bytes)
        .map_err(|e| format!("the new version could not be written to {}: {e}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)) {
            log::warn!("update: could not mark {} as a program: {e}", staged.display());
        }
    }
    let _ = std::fs::remove_file(&kept);
    if let Err(e) = std::fs::copy(target, &kept) {
        let _ = std::fs::remove_file(&staged);
        return Err(format!("the current version could not be set aside, so nothing changed: {e}"));
    }
    if let Err(e) = std::fs::rename(&staged, target) {
        let _ = std::fs::remove_file(&staged);
        let _ = std::fs::remove_file(&kept);
        return Err(format!("the new version could not be put in place, so nothing changed: {e}"));
    }
    Ok(())
}

/// Clear the .old files a previous update left beside these programs.
pub fn sweep_previous() {
    let Some(exe) = running_exe() else { return };
    if let Some(dir) = exe.parent() {
        for name in ["ayatsuri", "ayatsuri-launcher", "ayatsuri.exe", "ayatsuri-launcher.exe"] {
            if let Some(old) = previous_beside(&dir.join(name)) {
                remove_previous(&old);
            }
        }
    }
    if let Some(file) = std::env::var_os("APPIMAGE").map(PathBuf::from)
        && let Some(old) = previous_beside(&file)
    {
        remove_previous(&old);
    }
}

fn remove_previous(old: &Path) {
    match std::fs::remove_file(old) {
        Ok(()) => log::info!("update: removed the previous version {}", old.display()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            log::warn!("update: could not remove the previous version {}: {e}", old.display())
        }
    }
}

/// Start the checked installer quietly and detached; the caller then exits
/// so Windows lets the installer replace the running program. The bytes are
/// checked again first: nothing unchecked is ever run.
pub fn run_installer(path: &Path, sha256: &str) -> Result<(), String> {
    let bytes =
        std::fs::read(path).map_err(|e| format!("the installer could not be read back: {e}"))?;
    verify(&bytes, sha256)?;
    std::process::Command::new(path)
        .arg("/S")
        .current_dir(std::env::temp_dir())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("the installer would not start: {e}"))
}

/// Start the (new) launcher beside this program, for "Restart now".
pub fn start_launcher() -> Result<(), String> {
    let exe = std::env::var_os("APPIMAGE").map(PathBuf::from).or_else(|| {
        let dir = running_exe()?.parent()?.to_path_buf();
        let launcher = dir.join(format!("ayatsuri-launcher{}", std::env::consts::EXE_SUFFIX));
        Some(if launcher.exists() {
            launcher
        } else {
            dir.join(format!("ayatsuri{}", std::env::consts::EXE_SUFFIX))
        })
    });
    let exe = exe.ok_or("the new version could not be found to start")?;
    std::process::Command::new(exe)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("the new version would not start: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digests_come_from_the_named_line_only() {
        let sums = "aaaa  other.zip\n\
                    0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef  a-linux-x86_64.tar.gz\n";
        assert_eq!(
            digest_for(sums, "a-linux-x86_64.tar.gz").as_deref(),
            Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
        );
        assert!(digest_for(sums, "other.zip").is_none(), "a short digest is not one");
        assert!(digest_for(sums, "missing").is_none());
    }

    #[test]
    fn a_mismatched_download_is_refused() {
        let good: String = Sha256::digest(b"hello").iter().map(|b| format!("{b:02x}")).collect();
        assert!(verify(b"hello", &good).is_ok());
        assert!(verify(b"hellO", &good).is_err());
    }

    #[test]
    fn program_files_is_matched_with_a_separator() {
        let roots = vec!["C:\\Program Files".to_string()];
        assert!(in_any("C:/Program Files/Ayatsuri2D/ayatsuri.exe", &roots));
        assert!(!in_any("C:/Program FilesX/ayatsuri.exe", &roots));
        assert!(!in_any("C:/Users/me/Apps/ayatsuri.exe", &roots));
    }

    #[test]
    fn a_tar_entry_is_found_by_its_ending_and_a_swap_keeps_the_old_file() {
        // a one-file tar built by hand
        let mut tar = vec![0u8; 512];
        let name = b"pkg/bin/ayatsuri";
        tar[..name.len()].copy_from_slice(name);
        let size = format!("{:011o}\0", 5);
        tar[124..136].copy_from_slice(size.as_bytes());
        tar[156] = b'0';
        let mut body = b"hello".to_vec();
        body.resize(512, 0);
        tar.extend(body);
        tar.extend(vec![0u8; 1024]);
        assert_eq!(entry_in_tar(&tar, "/bin/ayatsuri").as_deref(), Some(&b"hello"[..]));
        assert!(entry_in_tar(&tar, "/bin/ayatsuri-launcher").is_none());

        let dir = std::env::temp_dir().join(format!("ay-update-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let target = dir.join("ayatsuri");
        std::fs::write(&target, b"old").unwrap();
        stage_then_rename(&target, b"new").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert_eq!(std::fs::read(dir.join("ayatsuri.old")).unwrap(), b"old");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
