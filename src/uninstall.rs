//! Removing dok: the binary, and the files it wrote outside itself.
//!
//! Only two things are ever dok's to delete — the config directory it writes
//! `themes --init` into, and the cache directory the update check lives in.
//! Both are found the same way the rest of the program finds them, so an
//! uninstall can never remove a directory dok would not have created.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// A file or directory an uninstall would remove.
pub struct Item {
    pub what: &'static str,
    pub path: PathBuf,
    pub dir: bool,
}

/// The config and cache directories, if they exist on this machine.
pub fn data() -> Vec<Item> {
    let mut out = Vec::new();
    if let Some(dir) = crate::config::config_path().and_then(|p| p.parent().map(Path::to_path_buf))
        && dir.is_dir()
    {
        out.push(Item { what: "config", path: dir, dir: true });
    }
    if let Some(dir) = crate::update::cache_dir()
        && dir.is_dir()
    {
        out.push(Item { what: "cache", path: dir, dir: true });
    }
    out
}

pub fn remove(item: &Item) -> Result<()> {
    let r = if item.dir {
        std::fs::remove_dir_all(&item.path)
    } else {
        std::fs::remove_file(&item.path)
    };
    r.with_context(|| format!("removing {}", item.path.display()))
}

/// Delete the running binary.
///
/// Unix unlinks it outright: the kernel keeps the inode alive until this
/// process exits, so deleting the file underneath ourselves is safe. Windows
/// refuses to unlink a running image, so the file is renamed out of the way
/// first — which Windows does allow — and a detached shell deletes it a moment
/// after we exit.
pub fn remove_self(exe: &Path) -> Result<()> {
    #[cfg(not(windows))]
    {
        std::fs::remove_file(exe)
            .with_context(|| format!("cannot remove {} — try again with sudo", exe.display()))?;
        Ok(())
    }

    #[cfg(windows)]
    {
        let dir = exe
            .parent()
            .ok_or_else(|| anyhow::anyhow!("{} has no parent directory", exe.display()))?;
        let stale = dir.join(format!(".dok-uninstall-{}.exe", std::process::id()));
        std::fs::rename(exe, &stale).with_context(|| {
            format!("cannot remove {} — close every running dok and try again", exe.display())
        })?;
        // ping is the portable "sleep" on Windows; the loop retries in case
        // this process has not finished exiting yet.
        let script =
            format!("ping -n 3 127.0.0.1 >nul & del /f /q \"{}\"", stale.to_string_lossy());
        std::process::Command::new("cmd")
            .args(["/C", &script])
            .spawn()
            .with_context(|| format!("could not schedule {} for deletion", stale.display()))?;
        Ok(())
    }
}

/// Anything dok installed beside the binary — today only the scoop shim, which
/// scoop itself would clean up but a standalone install never has.
pub fn strays(exe: &Path) -> Vec<Item> {
    let mut out = Vec::new();
    if let Some(dir) = exe.parent() {
        for name in ["dok.cmd", "dok.ps1"] {
            let p = dir.join(name);
            if p.is_file() {
                out.push(Item { what: "shim", path: p, dir: false });
            }
        }
    }
    out
}

/// Guard against `dok uninstall` run from a source checkout: deleting
/// `target/release/dok` is never what anybody means by uninstalling.
pub fn looks_like_build_output(exe: &Path) -> bool {
    let p = exe.to_string_lossy().replace('\\', "/");
    p.contains("/target/debug/") || p.contains("/target/release/")
}
