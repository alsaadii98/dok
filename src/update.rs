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

pub const REPO: &str = "alsaadii98/dok";

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

/// The directory dok keeps its cached update check in.
pub fn cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("dok"))
}

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

/// How dok was installed, which decides whether it can replace or remove
/// itself.
pub enum Install {
    /// A plain binary dok can overwrite or delete in place.
    Standalone(PathBuf),
    /// Owned by something else; the strings are that manager's commands.
    Managed {
        by: &'static str,
        /// The command that upgrades dok.
        cmd: String,
        /// The command that removes dok.
        remove: String,
    },
    Unknown,
}

pub fn detect() -> Install {
    let Ok(exe) = std::env::current_exe() else { return Install::Unknown };
    let exe = exe.canonicalize().unwrap_or(exe);
    let path = exe.to_string_lossy().to_string();
    // Windows paths are matched with the separator normalised, so one set of
    // patterns covers both.
    let slashed = path.replace('\\', "/");

    if slashed.contains("/Cellar/")
        || slashed.contains("/homebrew/")
        || slashed.contains("/linuxbrew/")
    {
        return Install::Managed {
            by: "homebrew",
            cmd: "brew upgrade dok".into(),
            remove: "brew uninstall dok".into(),
        };
    }
    if slashed.starts_with("/nix/store/") {
        return Install::Managed {
            by: "nix",
            cmd: "nix profile upgrade dok".into(),
            remove: "nix profile remove dok".into(),
        };
    }
    if slashed.contains("/.cargo/bin/") {
        return Install::Managed {
            by: "cargo",
            cmd: "cargo install dok-cli --force".into(),
            remove: "cargo uninstall dok-cli".into(),
        };
    }
    if slashed.contains("/scoop/apps/") || slashed.contains("/scoop/shims/") {
        return Install::Managed {
            by: "scoop",
            cmd: "scoop update dok".into(),
            remove: "scoop uninstall dok".into(),
        };
    }
    if slashed.contains("/WinGet/Packages/") {
        return Install::Managed {
            by: "winget",
            cmd: "winget upgrade dok".into(),
            remove: "winget uninstall dok".into(),
        };
    }
    if let Some(m) = system_package(&path) {
        return m;
    }
    Install::Standalone(exe)
}

/// Ask the system package managers whether they own this path.
fn system_package(path: &str) -> Option<Install> {
    let ok = |cmd: &str, args: &[&str]| -> bool {
        Command::new(cmd)
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    };
    // The package name is worth asking for rather than assuming: the AUR ships
    // both `dok` and `dok-bin`, and removing the wrong one does nothing.
    let owner = |cmd: &str, args: &[&str]| -> Option<String> {
        let out = Command::new(cmd).args(args).output().ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let first = text.lines().next()?.trim();
        // `dpkg -S` answers "dok: /usr/bin/dok"; `pacman -Qo` answers
        // "/usr/bin/dok is owned by dok 0.1.3".
        let name = if let Some((pkg, _)) = first.split_once(':') {
            pkg.trim().to_string()
        } else if let Some((_, rest)) = first.split_once(" is owned by ") {
            rest.split_whitespace().next()?.to_string()
        } else {
            first.split_whitespace().next()?.to_string()
        };
        (!name.is_empty()).then_some(name)
    };

    if ok("apk", &["info", "-e", "dok"]) {
        return Some(Install::Managed {
            by: "apk",
            cmd: "apk add --allow-untrusted dok-<arch>.apk  (download it from the release)".into(),
            remove: "sudo apk del dok".into(),
        });
    }
    if let Some(pkg) = owner("pacman", &["-Qoq", path]) {
        return Some(Install::Managed {
            by: "pacman",
            cmd: format!("pacman -Syu {pkg}"),
            remove: format!("sudo pacman -Rns {pkg}"),
        });
    }
    if let Some(pkg) = owner("dpkg", &["-S", path]) {
        return Some(Install::Managed {
            by: "dpkg",
            cmd: "sudo dpkg -i dok_amd64.deb  (download it from the release)".into(),
            remove: format!("sudo dpkg -r {pkg}"),
        });
    }
    if let Some(pkg) = owner("rpm", &["-qf", path]) {
        return Some(Install::Managed {
            by: "rpm",
            cmd: "sudo rpm -U dok.x86_64.rpm  (download it from the release)".into(),
            remove: format!("sudo rpm -e {pkg}"),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The checksum that gates every self-update. Pinned to the published
    /// SHA-256 of a known input so a dependency bump cannot change it quietly.
    #[test]
    fn sha256_matches_the_reference_vector() {
        let dir = std::env::temp_dir().join(format!("dok-sha-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("abc");
        std::fs::write(&f, b"abc").unwrap();
        assert_eq!(
            sha256_file(&f).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
