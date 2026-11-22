//! Compose projects as compose sees them: what the files declare, set against
//! what is running.
//!
//! Everything comes from labels compose writes on each container
//! (`com.docker.compose.*`) plus, when the daemon is on this machine, the
//! compose files those labels point at. dok never runs `docker compose`.

use bollard::models::ContainerSummary;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use yaml_rust2::{Yaml, YamlLoader};

use crate::dk;

pub const CONFIG_FILES: &str = "com.docker.compose.project.config_files";
pub const WORKING_DIR: &str = "com.docker.compose.project.working_dir";
pub const ONEOFF: &str = "com.docker.compose.oneoff";
pub const NUMBER: &str = "com.docker.compose.container-number";

/// The services a project's compose files declare.
#[derive(Debug, Default, PartialEq)]
pub struct Declared {
    pub services: BTreeMap<String, DeclaredService>,
}

#[derive(Debug, Default, PartialEq)]
pub struct DeclaredService {
    /// `deploy.replicas` or `scale`; `None` when it is not a plain number
    /// (an `${VAR}` that only compose can resolve).
    pub replicas: Option<u32>,
    /// Profiles gate a service: it only starts when one of them is active.
    pub profiles: Vec<String>,
    /// Shown for a service with no container to read it from.
    pub image: Option<String>,
}

/// Why the files could not be checked, shown in place of the checks.
#[derive(Debug, PartialEq)]
pub enum Unchecked {
    /// The daemon is another machine; the paths in the labels are its paths.
    Remote,
    /// No container carried a config-files label.
    NoFiles,
    /// A file is gone or unreadable here.
    Unreadable(String),
}

/// Read and merge a project's compose files, following top-level `include`.
/// `files` is the label value: absolute paths, comma-separated.
pub fn load(files: &str) -> Result<Declared, String> {
    let mut declared = Declared::default();
    for path in files.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        merge_file(Path::new(path), &mut declared, 0)?;
    }
    Ok(declared)
}

fn merge_file(path: &Path, into: &mut Declared, depth: u8) -> Result<(), String> {
    // Includes can include; a cycle should fail, not hang.
    if depth > 8 {
        return Err(format!("{}: includes nest too deep", path.display()));
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let docs = YamlLoader::load_from_str(&text)
        .map_err(|e| format!("cannot parse {}: {e}", path.display()))?;
    let Some(doc) = docs.first() else { return Ok(()) };

    let base = path.parent().unwrap_or(Path::new("."));
    for inc in include_paths(doc) {
        merge_file(&base.join(inc), into, depth + 1)?;
    }
    merge_doc(doc, into);
    Ok(())
}

/// `include:` entries are a path, or a map whose `path` is a path or a list.
fn include_paths(doc: &Yaml) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in doc["include"].as_vec().into_iter().flatten() {
        match entry {
            Yaml::String(p) => out.push(PathBuf::from(p)),
            Yaml::Hash(_) => match &entry["path"] {
                Yaml::String(p) => out.push(PathBuf::from(p)),
                Yaml::Array(ps) => {
                    out.extend(ps.iter().filter_map(Yaml::as_str).map(PathBuf::from))
                }
                _ => {}
            },
            _ => {}
        }
    }
    out
}

/// Later files override earlier ones key by key, the way compose merges them.
pub fn merge_doc(doc: &Yaml, into: &mut Declared) {
    let Some(services) = doc["services"].as_hash() else { return };
    for (name, svc) in services {
        let Some(name) = name.as_str() else { continue };
        let entry = into
            .services
            .entry(name.to_string())
            .or_insert(DeclaredService { replicas: Some(1), ..Default::default() });
        if let Some(image) = svc["image"].as_str() {
            entry.image = Some(image.to_string());
        }
        // `scale` is the older spelling; `deploy.replicas` wins when both are set.
        for key in [&svc["scale"], &svc["deploy"]["replicas"]] {
            match key {
                Yaml::BadValue => {}
                Yaml::Integer(n) => entry.replicas = u32::try_from(*n).ok(),
                _ => entry.replicas = None,
            }
        }
        if let Some(ps) = svc["profiles"].as_vec() {
            entry.profiles = ps.iter().filter_map(Yaml::as_str).map(str::to_string).collect();
        }
    }
}

/// How one service stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum State {
    /// Declared, but no container exists.
    Missing,
    /// Containers exist, none running.
    Down,
    /// Some replicas running, fewer than declared.
    Degraded,
    /// Running, not declared in the files: `up --remove-orphans` would remove it.
    Orphan,
    /// As declared.
    Running,
    /// Declared behind a profile that is not active, and not running.
    Inactive,
}

impl State {
    /// States that mean the project is not as declared.
    pub fn is_issue(self) -> bool {
        matches!(self, State::Missing | State::Down | State::Degraded | State::Orphan)
    }
    pub fn as_str(self) -> &'static str {
        match self {
            State::Missing => "missing",
            State::Down => "down",
            State::Degraded => "degraded",
            State::Orphan => "orphan",
            State::Running => "running",
            State::Inactive => "inactive",
        }
    }
}

pub struct Service<'a> {
    pub name: String,
    pub state: State,
    /// Replicas the files ask for; `None` when they could not be read or the
    /// count is not a literal number.
    pub expected: Option<u32>,
    pub profiles: Vec<String>,
    /// The running image, or the declared one when nothing runs.
    pub image: String,
    /// Replicas, ordered by container number.
    pub containers: Vec<&'a ContainerSummary>,
}

impl Service<'_> {
    pub fn running(&self) -> usize {
        self.containers.iter().filter(|c| dk::state_of(c) == "running").count()
    }
    pub fn unhealthy(&self) -> usize {
        self.containers
            .iter()
            .filter(|c| {
                dk::state_of(c) == "running" && dk::health_of(c).as_deref() == Some("unhealthy")
            })
            .count()
    }
}

pub struct Project<'a> {
    pub name: String,
    pub working_dir: Option<String>,
    pub config_files: Option<String>,
    pub services: Vec<Service<'a>>,
    /// `docker compose run` containers: not replicas of anything.
    pub oneoffs: Vec<&'a ContainerSummary>,
    /// Set when the files were not read, so missing and orphan services
    /// could not be told apart from the rest.
    pub unchecked: Option<Unchecked>,
}

impl Project<'_> {
    pub fn issues(&self) -> BTreeMap<State, usize> {
        let mut out = BTreeMap::new();
        for s in self.services.iter().filter(|s| s.state.is_issue()) {
            *out.entry(s.state).or_insert(0) += 1;
        }
        out
    }
    pub fn unhealthy(&self) -> usize {
        self.services.iter().map(Service::unhealthy).sum()
    }
    /// Services that should be up: declared ones outside an inactive profile.
    pub fn wanted(&self) -> usize {
        self.services.iter().filter(|s| !matches!(s.state, State::Inactive | State::Orphan)).count()
    }
    pub fn up(&self) -> usize {
        self.services.iter().filter(|s| s.state == State::Running).count()
    }
    pub fn healthy(&self) -> bool {
        self.issues().is_empty() && self.unhealthy() == 0
    }
}

/// Every compose project among `containers`, by name.
pub fn group(containers: &[ContainerSummary]) -> BTreeMap<String, Vec<&ContainerSummary>> {
    let mut out: BTreeMap<String, Vec<&ContainerSummary>> = BTreeMap::new();
    for c in containers {
        if let Some(p) = dk::label(c, dk::COMPOSE_PROJECT).filter(|p| !p.is_empty()) {
            out.entry(p.to_string()).or_default().push(c);
        }
    }
    out
}

/// Set a project's containers against its declared services. `declared` is
/// `Err` when the files were not read, and the view falls back to the
/// containers alone.
pub fn build<'a>(
    name: &str,
    containers: &[&'a ContainerSummary],
    declared: Result<Declared, Unchecked>,
) -> Project<'a> {
    let label = |key| containers.iter().find_map(|c| dk::label(c, key)).map(str::to_string);
    let oneoff = |c: &ContainerSummary| dk::label(c, ONEOFF) == Some("True");

    let mut by_service: BTreeMap<String, Vec<&ContainerSummary>> = BTreeMap::new();
    let mut oneoffs = Vec::new();
    for c in containers {
        if oneoff(c) {
            oneoffs.push(*c);
        } else {
            let svc = dk::label(c, dk::COMPOSE_SERVICE).unwrap_or("").to_string();
            by_service.entry(svc).or_default().push(*c);
        }
    }
    for list in by_service.values_mut() {
        list.sort_by_key(|c| dk::label(c, NUMBER).and_then(|n| n.parse::<u32>().ok()));
    }

    let (decl, unchecked) = match declared {
        Ok(d) => (Some(d), None),
        Err(u) => (None, Some(u)),
    };

    let mut services = Vec::new();
    let mut names: Vec<String> = by_service.keys().cloned().collect();
    if let Some(d) = &decl {
        names.extend(d.services.keys().cloned());
    }
    names.sort();
    names.dedup();

    for name in names {
        let list = by_service.remove(&name).unwrap_or_default();
        let spec = decl.as_ref().and_then(|d| d.services.get(&name));
        let running = list.iter().filter(|c| dk::state_of(c) == "running").count();
        let expected = match (&decl, spec) {
            (Some(_), Some(s)) => s.replicas,
            // Without the files, the containers compose made are the target.
            (None, _) => Some(list.len() as u32),
            (Some(_), None) => None,
        };
        let state = match (&decl, spec) {
            (Some(_), None) => State::Orphan,
            (_, Some(s)) if list.is_empty() && !s.profiles.is_empty() => State::Inactive,
            (_, Some(s)) if list.is_empty() && s.replicas == Some(0) => State::Running,
            _ if list.is_empty() => State::Missing,
            _ if running == 0 => State::Down,
            _ if expected.is_some_and(|e| (running as u32) < e) => State::Degraded,
            _ => State::Running,
        };
        services.push(Service {
            name,
            state,
            expected,
            profiles: spec.map(|s| s.profiles.clone()).unwrap_or_default(),
            image: list
                .first()
                .and_then(|c| c.image.clone())
                .or_else(|| spec.and_then(|s| s.image.clone()))
                .unwrap_or_default(),
            containers: list,
        });
    }

    Project {
        name: name.to_string(),
        working_dir: label(WORKING_DIR),
        config_files: label(CONFIG_FILES),
        services,
        oneoffs,
        unchecked,
    }
}

/// What to check a project against: its files when the daemon is local and
/// they can be read, otherwise the reason they were not.
pub fn declared_for(name: &str, containers: &[&ContainerSummary]) -> Result<Declared, Unchecked> {
    if crate::demo::enabled() {
        return crate::demo::compose_declared(name).ok_or(Unchecked::NoFiles);
    }
    if !dk::is_local() {
        return Err(Unchecked::Remote);
    }
    let files = containers
        .iter()
        .find_map(|c| dk::label(c, CONFIG_FILES))
        .filter(|f| !f.is_empty())
        .ok_or(Unchecked::NoFiles)?;
    load(files).map_err(Unchecked::Unreadable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bollard::models::{ContainerSummary, ContainerSummaryStateEnum as S};
    use std::collections::HashMap;

    fn ct(service: &str, n: u32, state: S) -> ContainerSummary {
        let mut labels = HashMap::from([
            (dk::COMPOSE_PROJECT.to_string(), "shop".to_string()),
            (dk::COMPOSE_SERVICE.to_string(), service.to_string()),
            (NUMBER.to_string(), n.to_string()),
            (ONEOFF.to_string(), "False".to_string()),
            (WORKING_DIR.to_string(), "/srv/shop".to_string()),
        ]);
        if service == "run" {
            labels.insert(ONEOFF.to_string(), "True".to_string());
        }
        ContainerSummary {
            id: Some(format!("{service}{n}")),
            names: Some(vec![format!("/shop-{service}-{n}")]),
            state: Some(state),
            labels: Some(labels),
            ..Default::default()
        }
    }

    fn decl(yaml: &str) -> Declared {
        let mut d = Declared::default();
        merge_doc(&YamlLoader::load_from_str(yaml).unwrap()[0], &mut d);
        d
    }

    fn states(p: &Project) -> Vec<(String, State)> {
        p.services.iter().map(|s| (s.name.clone(), s.state)).collect()
    }

    #[test]
    fn replicas_come_from_deploy_or_scale() {
        let d = decl(
            "services:\n  a: {image: x}\n  b: {scale: 3}\n  c: {deploy: {replicas: 2}}\n  \
             d: {scale: 5, deploy: {replicas: 4}}\n  e: {deploy: {replicas: '${N:-2}'}}\n  \
             f: {profiles: [debug]}\nx-common: &c {}\n",
        );
        let r = |s: &str| d.services[s].replicas;
        assert_eq!(
            (r("a"), r("b"), r("c"), r("d"), r("e")),
            (Some(1), Some(3), Some(2), Some(4), None)
        );
        assert_eq!(d.services["f"].profiles, vec!["debug"]);
        assert_eq!(d.services.len(), 6, "x- extension keys are not services");
    }

    /// Later files override earlier ones, as `-f a -f b` does.
    #[test]
    fn later_files_override_earlier_ones() {
        let mut d = decl("services:\n  api: {deploy: {replicas: 1}}\n");
        merge_doc(
            &YamlLoader::load_from_str("services:\n  api: {deploy: {replicas: 3}}\n  extra: {}\n")
                .unwrap()[0],
            &mut d,
        );
        assert_eq!(d.services["api"].replicas, Some(3));
        assert!(d.services.contains_key("extra"));
    }

    #[test]
    fn each_service_is_set_against_its_declaration() {
        let cs = [
            ct("api", 1, S::RUNNING),
            ct("worker", 1, S::RUNNING),
            ct("worker", 2, S::EXITED),
            ct("cron", 1, S::EXITED),
            ct("legacy", 1, S::RUNNING),
            ct("run", 1, S::RUNNING),
        ];
        let refs: Vec<&ContainerSummary> = cs.iter().collect();
        let d = decl(
            "services:\n  api: {}\n  worker: {deploy: {replicas: 2}}\n  cron: {}\n  \
             db: {}\n  debug: {profiles: [debug]}\n",
        );
        let p = build("shop", &refs, Ok(d));
        assert_eq!(
            states(&p),
            vec![
                ("api".into(), State::Running),
                ("cron".into(), State::Down),
                ("db".into(), State::Missing),
                ("debug".into(), State::Inactive),
                ("legacy".into(), State::Orphan),
                ("worker".into(), State::Degraded),
            ]
        );
        assert_eq!(p.oneoffs.len(), 1, "compose run containers are not replicas");
        assert_eq!(p.working_dir.as_deref(), Some("/srv/shop"));
        assert_eq!(p.wanted(), 4, "neither the orphan nor the inactive profile");
        assert_eq!(p.up(), 1);
        let issues = p.issues();
        assert_eq!(issues.get(&State::Orphan), Some(&1));
        assert_eq!(issues.get(&State::Missing), Some(&1));
        assert!(!issues.contains_key(&State::Inactive));
    }

    #[test]
    fn replicas_are_ordered_by_container_number() {
        let cs = [ct("w", 3, S::RUNNING), ct("w", 1, S::RUNNING), ct("w", 2, S::RUNNING)];
        let refs: Vec<&ContainerSummary> = cs.iter().collect();
        let p = build("shop", &refs, Err(Unchecked::Remote));
        let ids: Vec<_> = p.services[0].containers.iter().map(|c| c.id.clone().unwrap()).collect();
        assert_eq!(ids, ["w1", "w2", "w3"]);
    }

    /// Without the files there is nothing to call missing or orphaned; the
    /// containers compose made are the target.
    #[test]
    fn without_files_only_container_state_counts() {
        let cs = [ct("api", 1, S::RUNNING), ct("worker", 1, S::EXITED)];
        let refs: Vec<&ContainerSummary> = cs.iter().collect();
        let p = build("shop", &refs, Err(Unchecked::Remote));
        assert_eq!(
            states(&p),
            vec![("api".into(), State::Running), ("worker".into(), State::Down)]
        );
        assert_eq!(p.unchecked, Some(Unchecked::Remote));
        assert_eq!(p.services[0].expected, Some(1));
    }

    #[test]
    fn a_service_scaled_to_zero_is_as_declared() {
        let p = build("shop", &[], Ok(decl("services:\n  off: {deploy: {replicas: 0}}\n")));
        assert_eq!(states(&p), vec![("off".into(), State::Running)]);
    }

    #[test]
    fn files_and_includes_are_read_from_disk() {
        let dir = std::env::temp_dir().join(format!("dok-compose-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(
            dir.join("compose.yaml"),
            "include:\n  - sub/a.yaml\n  - path: [sub/b.yaml]\nservices:\n  web: {}\n",
        )
        .unwrap();
        std::fs::write(dir.join("sub/a.yaml"), "services:\n  db: {}\n").unwrap();
        std::fs::write(dir.join("sub/b.yaml"), "services:\n  cache: {scale: 2}\n").unwrap();
        std::fs::write(dir.join("override.yaml"), "services:\n  web: {scale: 3}\n").unwrap();

        let files = format!(
            "{},{}",
            dir.join("compose.yaml").display(),
            dir.join("override.yaml").display()
        );
        let d = load(&files).unwrap();
        let names: Vec<&str> = d.services.keys().map(String::as_str).collect();
        assert_eq!(names, ["cache", "db", "web"]);
        assert_eq!(d.services["web"].replicas, Some(3));
        assert_eq!(d.services["cache"].replicas, Some(2));

        let err = load(&dir.join("gone.yaml").display().to_string()).unwrap_err();
        assert!(err.starts_with("cannot read"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
