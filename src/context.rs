//! Docker contexts: `dok --context prod ps` reads the same daemon
//! `docker --context prod ps` would.
//!
//! Contexts live where the docker CLI keeps them, and are resolved in the
//! order docker/cli's `resolveContextName` uses:
//!
//!   1. `--context`
//!   2. `DOCKER_HOST` set   -> the default context, which honours it
//!   3. `DOCKER_CONTEXT`
//!   4. `currentContext` in `$DOCKER_CONFIG/config.json`
//!   5. the default context
//!
//! On disk a context `NAME` is `contexts/meta/<sha256(NAME)>/meta.json`, with
//! any TLS material in `contexts/tls/<sha256(NAME)>/docker/{ca,cert,key}.pem`.

use anyhow::{Context as _, Result, bail};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The context name docker reserves for "whatever DOCKER_HOST or the local
/// socket says". It never has a meta.json.
pub const DEFAULT: &str = "default";

/// Where to connect.
#[derive(Debug, PartialEq, Eq)]
pub enum Target {
    /// The default context: DOCKER_HOST if set, else the local socket.
    Default,
    /// A named context with its own endpoint.
    Named(Endpoint),
}

#[derive(Debug, PartialEq, Eq)]
pub struct Endpoint {
    pub name: String,
    pub host: String,
    /// The context's TLS directory, when it carries certificates.
    pub tls: Option<PathBuf>,
    pub skip_tls_verify: bool,
}

/// The context in use, kept for error messages: a failure against `prod`
/// should say it was `prod`.
static ACTIVE: OnceLock<String> = OnceLock::new();

pub fn active() -> Option<&'static str> {
    ACTIVE.get().map(String::as_str)
}

/// Resolve from the real environment and config directory.
pub fn resolve(flag: Option<&str>) -> Result<Target> {
    let target = resolve_in(
        flag,
        std::env::var("DOCKER_HOST").ok().filter(|v| !v.is_empty()),
        std::env::var("DOCKER_CONTEXT").ok().filter(|v| !v.is_empty()),
        &config_dir(),
    )?;
    if let Target::Named(ep) = &target {
        let _ = ACTIVE.set(format!("{} ({})", ep.name, ep.host));
    }
    Ok(target)
}

/// Resolution with its inputs passed in, so the precedence can be tested
/// without touching the process environment.
pub fn resolve_in(
    flag: Option<&str>,
    docker_host: Option<String>,
    docker_context: Option<String>,
    dir: &Path,
) -> Result<Target> {
    let name = if let Some(f) = flag {
        f.to_string()
    } else if docker_host.is_some() {
        return Ok(Target::Default);
    } else if let Some(c) = docker_context {
        c
    } else if let Some(c) = current_context(dir) {
        c
    } else {
        return Ok(Target::Default);
    };

    if name == DEFAULT {
        return Ok(Target::Default);
    }
    load(dir, &name).map(Target::Named)
}

/// `$DOCKER_CONFIG`, else `~/.docker`, which is where the docker CLI looks.
fn config_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("DOCKER_CONFIG").filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    let home =
        std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).unwrap_or_default();
    PathBuf::from(home).join(".docker")
}

fn current_context(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("config.json")).ok()?;
    let cfg: Value = serde_json::from_str(&text).ok()?;
    get(&cfg, "currentContext")?.as_str().filter(|s| !s.is_empty()).map(str::to_string)
}

/// The directory docker names a context's files after: the hex sha256 of
/// its name.
fn digest(name: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(name.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn load(dir: &Path, name: &str) -> Result<Endpoint> {
    let id = digest(name);
    let meta_path = dir.join("contexts/meta").join(&id).join("meta.json");
    let text = match std::fs::read_to_string(&meta_path) {
        Ok(t) => t,
        Err(_) => {
            let known = list(dir);
            if known.is_empty() {
                bail!(
                    "docker context `{name}` not found (no contexts exist; see `docker context ls`)"
                );
            }
            bail!("docker context `{name}` not found; known: {}", known.join(", "));
        }
    };
    let meta: Value = serde_json::from_str(&text)
        .with_context(|| format!("cannot parse {}", meta_path.display()))?;

    let ep = get(&meta, "endpoints")
        .and_then(|e| get(e, "docker"))
        .with_context(|| format!("docker context `{name}` has no docker endpoint"))?;
    let host = get(ep, "host")
        .and_then(Value::as_str)
        .filter(|h| !h.is_empty())
        .with_context(|| format!("docker context `{name}` has no host"))?
        .to_string();
    let skip_tls_verify = get(ep, "skipTLSVerify").and_then(Value::as_bool).unwrap_or(false);

    let tls_dir = dir.join("contexts/tls").join(&id).join("docker");
    let tls = tls_dir.join("ca.pem").exists().then_some(tls_dir);

    Ok(Endpoint { name: name.to_string(), host, tls, skip_tls_verify })
}

/// Names of every stored context, for "did you mean" errors.
fn list(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir.join("contexts/meta"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| std::fs::read_to_string(e.path().join("meta.json")).ok())
        .filter_map(|t| serde_json::from_str::<Value>(&t).ok())
        .filter_map(|m| get(&m, "name").and_then(Value::as_str).map(str::to_string))
        .collect();
    names.sort();
    names
}

/// Key lookup ignoring case. docker/cli writes these files from more than one
/// struct, and Go's decoder matches keys case-insensitively, so both `Host`
/// and `host` are real.
fn get<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    v.as_object()?.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway $DOCKER_CONFIG holding the given contexts.
    struct Fake(PathBuf);
    impl Fake {
        fn new(tag: &str) -> Self {
            let d = std::env::temp_dir().join(format!("dok-ctx-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            Fake(d)
        }
        fn context(&self, name: &str, meta: &str) -> &Self {
            let d = self.0.join("contexts/meta").join(digest(name));
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("meta.json"), meta).unwrap();
            self
        }
        fn current(&self, name: &str) -> &Self {
            std::fs::write(self.0.join("config.json"), format!(r#"{{"currentContext":"{name}"}}"#))
                .unwrap();
            self
        }
    }
    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn prod(host: &str) -> String {
        format!(
            r#"{{"Name":"prod","Metadata":{{}},"Endpoints":{{"docker":{{"Host":"{host}","SkipTLSVerify":false}}}}}}"#
        )
    }

    fn host(t: Target) -> String {
        match t {
            Target::Named(e) => e.host,
            Target::Default => DEFAULT.into(),
        }
    }

    /// The directory name is docker's, so a context made by `docker context
    /// create` is found. This is sha256("default"), a known value.
    #[test]
    fn context_dirs_are_named_by_sha256() {
        assert_eq!(
            digest("default"),
            "37a8eec1ce19687d132fe29051dca629d164e2c4958ba141d5f4133a33f0688f"
        );
    }

    #[test]
    fn the_flag_beats_everything() {
        let f = Fake::new("flag");
        f.context("prod", &prod("ssh://me@prod")).current("other");
        let t = resolve_in(Some("prod"), Some("tcp://x:1".into()), Some("other".into()), &f.0);
        assert_eq!(host(t.unwrap()), "ssh://me@prod");
    }

    /// DOCKER_HOST means "use this host", which docker models as the default
    /// context; it outranks DOCKER_CONTEXT and the config file.
    #[test]
    fn docker_host_outranks_docker_context_and_config() {
        let f = Fake::new("host");
        f.context("prod", &prod("ssh://me@prod")).current("prod");
        let t = resolve_in(None, Some("tcp://x:1".into()), Some("prod".into()), &f.0).unwrap();
        assert_eq!(t, Target::Default);
    }

    #[test]
    fn docker_context_outranks_the_config_file() {
        let f = Fake::new("env");
        f.context("prod", &prod("ssh://me@prod"))
            .context("staging", &prod("ssh://me@staging").replace("prod", "staging"))
            .current("staging");
        let t = resolve_in(None, None, Some("prod".into()), &f.0).unwrap();
        assert_eq!(host(t), "ssh://me@prod");
    }

    #[test]
    fn the_config_file_is_used_when_nothing_else_is_set() {
        let f = Fake::new("cfg");
        f.context("prod", &prod("ssh://me@prod")).current("prod");
        assert_eq!(host(resolve_in(None, None, None, &f.0).unwrap()), "ssh://me@prod");
    }

    #[test]
    fn nothing_set_means_the_default_context() {
        let f = Fake::new("none");
        assert_eq!(resolve_in(None, None, None, &f.0).unwrap(), Target::Default);
    }

    #[test]
    fn the_name_default_never_reads_a_file() {
        let f = Fake::new("default");
        assert_eq!(resolve_in(Some("default"), None, None, &f.0).unwrap(), Target::Default);
        f.current("default");
        assert_eq!(resolve_in(None, None, None, &f.0).unwrap(), Target::Default);
    }

    #[test]
    fn an_unknown_context_names_the_ones_that_exist() {
        let f = Fake::new("unknown");
        f.context("prod", &prod("ssh://me@prod"));
        let err = resolve_in(Some("prdo"), None, None, &f.0).unwrap_err().to_string();
        assert!(err.contains("`prdo` not found"), "{err}");
        assert!(err.contains("known: prod"), "{err}");
    }

    /// Both casings are written in the wild; either must load.
    #[test]
    fn meta_keys_are_matched_without_regard_to_case() {
        let f = Fake::new("case");
        f.context(
            "lower",
            r#"{"name":"lower","endpoints":{"docker":{"host":"unix:///x.sock","skipTLSVerify":true}}}"#,
        );
        match resolve_in(Some("lower"), None, None, &f.0).unwrap() {
            Target::Named(e) => {
                assert_eq!(e.host, "unix:///x.sock");
                assert!(e.skip_tls_verify);
            }
            t => panic!("{t:?}"),
        }
    }

    #[test]
    fn tls_material_is_found_beside_the_context() {
        let f = Fake::new("tls");
        f.context("prod", &prod("tcp://prod:2376"));
        let tls = f.0.join("contexts/tls").join(digest("prod")).join("docker");
        std::fs::create_dir_all(&tls).unwrap();
        for p in ["ca.pem", "cert.pem", "key.pem"] {
            std::fs::write(tls.join(p), "x").unwrap();
        }
        match resolve_in(Some("prod"), None, None, &f.0).unwrap() {
            Target::Named(e) => assert_eq!(e.tls.as_deref(), Some(tls.as_path())),
            t => panic!("{t:?}"),
        }
    }
}
