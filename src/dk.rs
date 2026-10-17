//! Docker daemon access and the small view structs the commands render.

use anyhow::{Context, Result};
use bollard::Docker;
use bollard::models::{ContainerSummary, ImageHistoryResponseItem, ImageSummary, Network, Volume};
use bollard::query_parameters::{
    ListContainersOptionsBuilder, ListImagesOptionsBuilder, ListNetworksOptions, ListVolumesOptions,
};

pub const COMPOSE_PROJECT: &str = "com.docker.compose.project";
pub const COMPOSE_SERVICE: &str = "com.docker.compose.service";

/// The `--context` flag, set once at startup.
static CONTEXT_FLAG: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();

pub fn set_context(flag: Option<String>) {
    let _ = CONTEXT_FLAG.set(flag);
}

/// Seconds to wait on a request, as bollard's own defaults use.
const TIMEOUT: u64 = 120;

pub fn connect() -> Result<Docker> {
    // Demo mode answers every query from fixtures, but the commands still need
    // a client value to pass around. Build an HTTP one pointed at a dead port:
    // it constructs without touching the filesystem, so `--demo` works on a
    // machine that has no docker socket at all.
    if crate::demo::enabled() {
        return Docker::connect_with_http("http://127.0.0.1:1", 1, bollard::API_DEFAULT_VERSION)
            .context("building the offline demo client");
    }
    let flag = CONTEXT_FLAG.get().and_then(|f| f.as_deref());
    match crate::context::resolve(flag)? {
        crate::context::Target::Default => Docker::connect_with_defaults()
            .context("cannot reach the docker daemon (is it running? check DOCKER_HOST)"),
        crate::context::Target::Named(ep) => connect_endpoint(&ep)
            .with_context(|| format!("cannot use docker context `{}` ({})", ep.name, ep.host)),
    }
}

/// An ssh:// endpoint in the form bollard can dial.
///
/// bollard strips the scheme before handing the address to the openssh crate,
/// which only splits out a port when the scheme is still there, so
/// `ssh://me@host:2222` reaches ssh as a hostname of `host:2222`. Port 22 is
/// the default and can simply be dropped; any other port has to come from
/// ~/.ssh/config, which ssh reads either way.
fn ssh_host(host: &str) -> Result<String> {
    let rest = host.trim_start_matches("ssh://").trim_end_matches('/');
    let (userhost, port) = match rest.rsplit_once(':') {
        Some((h, p)) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => (h, Some(p)),
        _ => (rest, None),
    };
    match port {
        None | Some("22") => Ok(format!("ssh://{userhost}")),
        Some(p) => {
            let bare = userhost.rsplit('@').next().unwrap_or(userhost);
            anyhow::bail!(
                "ssh contexts on a port other than 22 are not supported yet ({host}).\n\
                 Give the host an alias in ~/.ssh/config and point the context at it:\n\n\
                 \x20 Host {bare}-docker\n\
                 \x20   HostName {bare}\n\
                 \x20   Port {p}\n\n\
                 \x20 docker context update <name> --docker host=ssh://{userhost}-docker",
            )
        }
    }
}

/// One named context's endpoint, by scheme. bollard connects lazily, so this
/// only fails on a malformed endpoint; an unreachable host fails on the first
/// request, and main() names the context in that error too.
fn connect_endpoint(ep: &crate::context::Endpoint) -> Result<Docker> {
    use bollard::API_DEFAULT_VERSION as V;
    let host = ep.host.as_str();
    if host.starts_with("ssh://") {
        #[cfg(unix)]
        return Ok(Docker::connect_with_ssh(&ssh_host(host)?, TIMEOUT, V, None)?);
        #[cfg(not(unix))]
        anyhow::bail!(
            "ssh:// contexts need dok on macOS or Linux; on Windows use a tcp:// context"
        );
    }
    if host.starts_with("tcp://") || host.starts_with("https://") || host.starts_with("http://") {
        if let Some(tls) = &ep.tls {
            if ep.skip_tls_verify {
                // bollard verifies against the context's CA and has no way to
                // turn that off, so this is a limit, not a policy.
                anyhow::bail!("contexts with SkipTLSVerify are not supported");
            }
            return Ok(Docker::connect_with_ssl(
                host,
                &tls.join("key.pem"),
                &tls.join("cert.pem"),
                &tls.join("ca.pem"),
                TIMEOUT,
                V,
            )?);
        }
        return Ok(Docker::connect_with_http(host, TIMEOUT, V)?);
    }
    #[cfg(unix)]
    if host.starts_with("unix://") {
        return Ok(Docker::connect_with_unix(host, TIMEOUT, V)?);
    }
    #[cfg(windows)]
    if host.starts_with("npipe://") {
        return Ok(Docker::connect_with_named_pipe(host, TIMEOUT, V)?);
    }
    anyhow::bail!("unsupported endpoint scheme in `{host}`")
}

pub async fn containers(docker: &Docker, all: bool) -> Result<Vec<ContainerSummary>> {
    if crate::demo::enabled() {
        let mut list = crate::demo::containers();
        if !all {
            list.retain(|c| state_of(c) == "running");
        }
        return Ok(list);
    }
    let opts = ListContainersOptionsBuilder::default().all(all).build();
    Ok(docker.list_containers(Some(opts)).await?)
}

pub async fn images(docker: &Docker, all: bool) -> Result<Vec<ImageSummary>> {
    if crate::demo::enabled() {
        return Ok(crate::demo::images());
    }
    let opts = ListImagesOptionsBuilder::default().all(all).build();
    Ok(docker.list_images(Some(opts)).await?)
}

pub async fn networks(docker: &Docker) -> Result<Vec<Network>> {
    if crate::demo::enabled() {
        return Ok(crate::demo::networks());
    }
    Ok(docker.list_networks(None::<ListNetworksOptions>).await?)
}

pub async fn volumes(docker: &Docker) -> Result<Vec<Volume>> {
    if crate::demo::enabled() {
        return Ok(crate::demo::volumes());
    }
    Ok(docker.list_volumes(None::<ListVolumesOptions>).await?.volumes.unwrap_or_default())
}

pub async fn df(docker: &Docker) -> Result<bollard::models::SystemDataUsageResponse> {
    if crate::demo::enabled() {
        return Ok(crate::demo::df());
    }
    Ok(docker.df(None::<bollard::query_parameters::DataUsageOptions>).await?)
}

pub async fn image_history(docker: &Docker, image: &str) -> Result<Vec<ImageHistoryResponseItem>> {
    if crate::demo::enabled() {
        return crate::demo::history(image);
    }
    docker
        .image_history(image)
        .await
        .with_context(|| format!("cannot read the history of `{image}`"))
}

/// Resolve a user-typed image reference to something the daemon will accept.
///
/// An exact `repo:tag` wins, then a bare repo on any tag, then an id prefix,
/// then a substring — so `postgres` finds `postgres:16-alpine` without also
/// matching every image that merely mentions it.
pub async fn resolve_image(docker: &Docker, needle: &str) -> Result<String> {
    let list = images(docker, false).await?;
    let tags: Vec<&String> = list
        .iter()
        .flat_map(|i| i.repo_tags.iter())
        .filter(|t| t.as_str() != "<none>:<none>")
        .collect();

    if let Some(t) = tags.iter().find(|t| t.as_str() == needle) {
        return Ok((*t).clone());
    }
    if let Some(t) = tags.iter().find(|t| t.split(':').next() == Some(needle)) {
        return Ok((*t).clone());
    }
    if let Some(i) = list.iter().find(|i| {
        i.id.strip_prefix("sha256:").unwrap_or(&i.id).starts_with(needle) && !needle.is_empty()
    }) {
        return Ok(i.id.clone());
    }
    if let Some(t) = tags.iter().find(|t| t.contains(needle)) {
        return Ok((*t).clone());
    }
    // Nothing local matched. The daemon may still know the reference, so hand
    // it over rather than refusing on our own guess.
    Ok(needle.to_string())
}

/// Resolve a user-typed name/id prefix to a concrete container name.
pub async fn resolve(docker: &Docker, needle: &str) -> Result<String> {
    let list = containers(docker, true).await?;
    // Exact name wins over a substring match, so `db` never picks `db-backup`.
    let exact = list.iter().find(|c| name_of(c) == needle);
    let fuzzy = list.iter().find(|c| {
        name_of(c).contains(needle)
            || c.id.as_deref().is_some_and(|id| id.starts_with(needle))
            || label(c, COMPOSE_SERVICE) == Some(needle)
    });
    exact.or(fuzzy).map(name_of).with_context(|| format!("no container matches `{needle}`"))
}

/// Container name without docker's leading `/`.
pub fn name_of(c: &ContainerSummary) -> String {
    c.names
        .as_ref()
        .and_then(|n| n.first())
        .map(|n| n.trim_start_matches('/').to_string())
        .unwrap_or_else(|| c.id.clone().unwrap_or_default().chars().take(12).collect())
}

pub fn label<'a>(c: &'a ContainerSummary, key: &str) -> Option<&'a str> {
    c.labels.as_ref()?.get(key).map(String::as_str)
}

pub fn state_of(c: &ContainerSummary) -> String {
    c.state.map(|s| s.to_string()).unwrap_or_default()
}

pub fn health_of(c: &ContainerSummary) -> Option<String> {
    let h = c.health.as_ref()?.status?;
    let s = h.to_string();
    if s.is_empty() || s == "none" { None } else { Some(s) }
}

/// Seconds since the container/image was created.
pub fn age_secs(created_unix: i64) -> i64 {
    chrono::Utc::now().timestamp() - created_unix
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_hosts_without_a_port_pass_through() {
        assert_eq!(ssh_host("ssh://me@prod").unwrap(), "ssh://me@prod");
        assert_eq!(ssh_host("ssh://prod").unwrap(), "ssh://prod");
    }

    /// Port 22 is ssh's default, so dropping it changes nothing and avoids
    /// the bollard bug.
    #[test]
    fn an_explicit_port_22_is_dropped() {
        assert_eq!(ssh_host("ssh://me@prod:22").unwrap(), "ssh://me@prod");
    }

    #[test]
    fn another_port_explains_the_ssh_config_workaround() {
        let err = ssh_host("ssh://me@prod.example.com:2222").unwrap_err().to_string();
        assert!(err.contains("Port 2222"), "{err}");
        assert!(err.contains("HostName prod.example.com"), "{err}");
    }
}
