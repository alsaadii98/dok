//! Update checking and self-replacement.
//!
//! The check is a single GitHub API call, cached for a day under
//! `~/.cache/dok/update.json`, so a normal command pays for it at most once
//! every 24 hours and never more than a couple of seconds. Set
//! `DOK_NO_UPDATE_CHECK=1` to turn it off entirely.
//!
//! Transport is `curl` or `wget` rather than a linked TLS stack: dok ships as
//! a static musl binary and every machine that runs docker already has one of
//! the two.

use anyhow::{Context, Result, anyhow, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const REPO: &str = "alsaadii98/cool-docker-commands";

/// The version this binary was built as.
pub fn current() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// `1.2.3` -> `(1, 2, 3)`; anything unparseable sorts as zero.
fn parts(v: &str) -> (u64, u64, u64) {
    let v = v.trim().trim_start_matches('v');
    let v = v.split(['-', '+']).next().unwrap_or(v);
    let mut it = v.split('.').map(|n| n.parse::<u64>().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0))
}

pub fn is_newer(candidate: &str, than: &str) -> bool {
    parts(candidate) > parts(than)
}

// ── transport ───────────────────────────────────────────────────────────────

fn have(cmd: &str) -> bool {
    Command::new(cmd)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Fetch a URL as text. `secs` bounds the whole transfer.
pub fn http_get(url: &str, secs: u32) -> Result<String> {
    let out = if have("curl") {
        Command::new("curl")
            .args(["-fsSL", "--max-time", &secs.to_string(), "-A", "dok", url])
            .output()
            .context("running curl")?
    } else if have("wget") {
        Command::new("wget")
            .args(["-q", "-O", "-", "-T", &secs.to_string(), url])
            .output()
            .context("running wget")?
    } else {
        bail!("neither curl nor wget is installed, so dok cannot reach GitHub");
    };
    if !out.status.success() {
        bail!("fetching {url} failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Download a URL to a file, with no timeout: release archives are megabytes.
pub fn http_download(url: &str, dest: &Path) -> Result<()> {
    let dest_s = dest.to_string_lossy().to_string();
    let out = if have("curl") {
        Command::new("curl").args(["-fsSL", "-A", "dok", "-o", &dest_s, url]).output()?
    } else if have("wget") {
        Command::new("wget").args(["-q", "-O", &dest_s, url]).output()?
    } else {
        bail!("neither curl nor wget is installed, so dok cannot download the release");
    };
    if !out.status.success() {
        bail!("downloading {url} failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// Latest published release version, without the `v`.
pub fn latest_version(secs: u32) -> Result<String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let body = http_get(&url, secs)?;
    let json: serde_json::Value =
        serde_json::from_str(&body).context("GitHub returned something that is not JSON")?;
    let tag = json
        .get("tag_name")
        .and_then(|t| t.as_str())
        .ok_or_else(|| anyhow!("no tag_name in the GitHub response"))?;
    Ok(tag.trim_start_matches('v').to_string())
}

// ── the once-a-day cache ────────────────────────────────────────────────────

const DAY: i64 = 24 * 60 * 60;

fn cache_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("dok").join("update.json"))
}

fn read_cache() -> Option<(i64, String)> {
    let text = std::fs::read_to_string(cache_path()?).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    Some((v.get("checked_at")?.as_i64()?, v.get("latest")?.as_str()?.to_string()))
}

pub fn write_cache(latest: &str) {
    let Some(path) = cache_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let now = chrono::Utc::now().timestamp();
    let body = serde_json::json!({ "checked_at": now, "latest": latest });
    let _ = std::fs::write(path, body.to_string());
}

/// The version to nag about, if any. Refreshes the cache at most daily and
/// stays quiet on every kind of failure — an update check must never be the
/// reason a command looks broken.
pub fn pending() -> Option<String> {
    if std::env::var_os("DOK_NO_UPDATE_CHECK").is_some() {
        return None;
    }
    // With nowhere to remember the answer, the check would run on every single
    // command. Better to stay quiet than to make dok slower for the privilege.
    cache_path()?;

    let now = chrono::Utc::now().timestamp();
    let latest = match read_cache() {
        Some((at, latest)) if now - at < DAY => latest,
        _ => match latest_version(2) {
            Ok(latest) => {
                write_cache(&latest);
                latest
            }
            // A failed check is cached as "nothing new", so an offline machine
            // tries once a day rather than once a command.
            Err(_) => {
                write_cache(current());
                return None;
            }
        },
    };
    is_newer(&latest, current()).then_some(latest)
}

// ── where this binary came from ─────────────────────────────────────────────

/// How dok was installed, which decides whether it can replace itself.
pub enum Install {
    /// A plain binary dok can overwrite in place.
    Standalone(PathBuf),
    /// Owned by something else; the string is the command that updates it.
    Managed {
        by: &'static str,
        cmd: String,
    },
    Unknown,
}

pub fn detect() -> Install {
    let Ok(exe) = std::env::current_exe() else { return Install::Unknown };
    let exe = exe.canonicalize().unwrap_or(exe);
    let path = exe.to_string_lossy().to_string();

    if path.contains("/Cellar/") || path.contains("/homebrew/") || path.contains("/linuxbrew/") {
        return Install::Managed { by: "homebrew", cmd: "brew upgrade dok".into() };
    }
    if path.starts_with("/nix/store/") {
        return Install::Managed { by: "nix", cmd: "nix profile upgrade dok".into() };
    }
    if path.contains("/.cargo/bin/") {
        return Install::Managed { by: "cargo", cmd: "cargo install dok-cli --force".into() };
    }
    if let Some(m) = system_package(&path) {
        return m;
    }
    Install::Standalone(exe)
}

/// Ask the system package managers whether they own this path.
fn system_package(path: &str) -> Option<Install> {
    let owned = |cmd: &str, args: &[&str]| -> bool {
        Command::new(cmd)
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    };
    if owned("apk", &["info", "-e", "dok"]) {
        return Some(Install::Managed {
            by: "apk",
            cmd: "apk add --allow-untrusted dok-<arch>.apk  (download it from the release)".into(),
        });
    }
    if owned("dpkg", &["-S", path]) {
        return Some(Install::Managed {
            by: "dpkg",
            cmd: "sudo dpkg -i dok_amd64.deb  (download it from the release)".into(),
        });
    }
    if owned("rpm", &["-qf", path]) {
        return Some(Install::Managed {
            by: "rpm",
            cmd: "sudo rpm -U dok.x86_64.rpm  (download it from the release)".into(),
        });
    }
    None
}

/// The rust target triple this binary was built for, which is also the name
/// of the release archive it should download.
pub fn target_triple() -> String {
    let arch = std::env::consts::ARCH;
    if cfg!(target_os = "macos") {
        return format!("{arch}-apple-darwin");
    }
    if cfg!(target_os = "windows") {
        return format!("{arch}-pc-windows-msvc");
    }
    let env = if cfg!(target_env = "musl") { "musl" } else { "gnu" };
    format!("{arch}-unknown-linux-{env}")
}

// ── replacing the binary ────────────────────────────────────────────────────

fn sha256_file(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path)?;
    let digest = Sha256::digest(&bytes);
    Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
}

/// Download `version`'s archive for this platform and move the binary over
/// `exe`. The download is checksum-verified against the release's `.sha256`
/// before anything is replaced.
pub fn replace(exe: &Path, version: &str) -> Result<()> {
    if cfg!(target_os = "windows") {
        bail!("self-update is not supported on Windows — use `scoop update dok`");
    }
    let triple = target_triple();
    let name = format!("dok-{version}-{triple}");
    let archive = format!("{name}.tar.gz");
    let base = format!("https://github.com/{REPO}/releases/download/v{version}");

    // Stage next to the binary so the final move is a rename on one
    // filesystem, which is atomic and cannot leave a half-written dok.
    let dir = exe.parent().ok_or_else(|| anyhow!("{} has no parent directory", exe.display()))?;
    let stage = dir.join(format!(".dok-update-{}", std::process::id()));
    std::fs::create_dir_all(&stage)
        .with_context(|| format!("cannot write to {} — try again with sudo", dir.display()))?;
    let _guard = Cleanup(stage.clone());

    let tarball = stage.join(&archive);
    http_download(&format!("{base}/{archive}"), &tarball)?;

    let want = http_get(&format!("{base}/{archive}.sha256"), 20)?.trim().to_lowercase();
    let got = sha256_file(&tarball)?;
    if !want.is_empty() && want != got {
        bail!("checksum mismatch for {archive}: expected {want}, got {got}");
    }

    let ok = Command::new("tar")
        .args(["xzf", &tarball.to_string_lossy(), "-C", &stage.to_string_lossy()])
        .status()
        .context("running tar")?;
    if !ok.success() {
        bail!("could not unpack {archive}");
    }

    let fresh = stage.join(&name).join("dok");
    if !fresh.exists() {
        bail!("{archive} did not contain dok");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fresh, std::fs::Permissions::from_mode(0o755))?;
    }
    std::fs::rename(&fresh, exe)
        .with_context(|| format!("cannot replace {} — try again with sudo", exe.display()))?;
    Ok(())
}

/// Removes the staging directory however `replace` returns.
struct Cleanup(PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
