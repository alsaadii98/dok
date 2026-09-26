//! `dok ports` — every published port, flat and sorted by host port.
//!
//! `dok ps` answers "what is running". This answers "who owns 5432?", which is
//! a different question and a worse fit for a per-container view: the ports are
//! spread down a column, wrapped, and interleaved with everything else. Here
//! there is one row per published binding, sorted by host port, so the lookup
//! is a glance or a `grep` rather than a scan.

use anyhow::Result;
use bollard::models::ContainerSummary;
use std::collections::HashMap;

use crate::dk;
use crate::json;
use crate::table::{Column, Table};
use crate::theme::{self, *};

/// One published binding: a host address and port mapped into a container.
struct Binding {
    /// Empty when bound to every interface. Docker reports that as `0.0.0.0`
    /// for v4 and `::` for v6; both mean "no restriction", so neither is worth
    /// a column of its own.
    host_ip: String,
    host_port: u16,
    container_port: u16,
    proto: String,
    /// Compose service name when there is one, else the container name.
    container: String,
    project: String,
    state: String,
    /// Another container publishes the same address, port and protocol.
    conflict: bool,
}

pub async fn run(all: bool) -> Result<()> {
    let docker = dk::connect()?;
    let list = dk::containers(&docker, all).await?;

    let mut rows = collect(&list);
    if json::enabled() {
        rows.sort_by(|a, b| {
            a.host_port
                .cmp(&b.host_port)
                .then_with(|| a.proto.cmp(&b.proto))
                .then_with(|| a.host_ip.cmp(&b.host_ip))
                .then_with(|| a.container.cmp(&b.container))
        });
        mark_conflicts(&mut rows);
        json::emit(&serde_json::json!({
            "ports": rows.iter().map(|b| serde_json::json!({
                // Empty means every interface; say so explicitly rather than
                // making a consumer guess what "" meant.
                "host_ip": if b.host_ip.is_empty() { serde_json::Value::Null } else { b.host_ip.as_str().into() },
                "host_port": b.host_port,
                "container_port": b.container_port,
                "protocol": b.proto,
                "container": b.container,
                "project": json::opt(&b.project),
                "state": b.state,
                "conflict": b.conflict,
            })).collect::<Vec<_>>(),
        }));
        return Ok(());
    }
    if rows.is_empty() {
        let msg = if all {
            "no published ports"
        } else {
            "no published ports on running containers (try -a)"
        };
        println!("{}", dim(msg));
        return Ok(());
    }

    // Host port is the lookup key, so it leads the sort. The rest only settles
    // ties into a stable order.
    rows.sort_by(|a, b| {
        a.host_port
            .cmp(&b.host_port)
            .then_with(|| a.proto.cmp(&b.proto))
            .then_with(|| a.host_ip.cmp(&b.host_ip))
            .then_with(|| a.container.cmp(&b.container))
    });
    mark_conflicts(&mut rows);

    // Binding to a specific address is rare and worth seeing when it happens.
    // A column of `0.0.0.0` on every row is exactly the noise dok exists to
    // remove, so the column only appears when some binding is narrower.
    let show_bind = rows.iter().any(|b| !b.host_ip.is_empty());

    let mut cols = vec![Column::left(""), Column::right("PORT")];
    if show_bind {
        cols.push(Column::left("BIND").flex(7).cap(20));
    }
    cols.extend([
        Column::left("PROTO"),
        Column::left("CONTAINER").flex(12).cap(30),
        Column::left("PROJECT").flex(8).cap(24),
        Column::right("TARGET"),
    ]);

    let mut t = Table::new(cols);
    for b in &rows {
        t.row(render(b, show_bind));
    }
    t.print();
    println!("{}", summary_line(&rows));
    Ok(())
}

/// Flatten every container's published ports into one list.
fn collect(list: &[ContainerSummary]) -> Vec<Binding> {
    let mut out: Vec<Binding> = Vec::new();
    for ct in list {
        let Some(ports) = &ct.ports else { continue };
        let name = crate::fmt::short_task_name(&dk::name_of(ct));
        let container = dk::label(ct, dk::COMPOSE_SERVICE).map(str::to_string).unwrap_or(name);
        let project = dk::label(ct, dk::COMPOSE_PROJECT).unwrap_or("").to_string();
        let state = dk::state_of(ct);

        for p in ports {
            // No public port means exposed-but-not-published. `dok ps` shows
            // those dimmed; here they would be answering a question nobody
            // asked, because nothing on the host can reach them.
            let Some(host_port) = p.public_port else { continue };
            let proto = p.typ.map(|t| t.to_string()).unwrap_or_else(|| "tcp".into());
            let host_ip = normalise_ip(p.ip.as_deref().unwrap_or(""));

            // Docker reports a wildcard binding twice, once for v4 (`0.0.0.0`)
            // and once for v6 (`::`). Both normalise to empty above, so this
            // collapses the pair into the single row a person means by it.
            let dup = out.iter().any(|b| {
                b.container == container
                    && b.host_ip == host_ip
                    && b.host_port == host_port
                    && b.container_port == p.private_port
                    && b.proto == proto
            });
            if dup {
                continue;
            }

            out.push(Binding {
                host_ip,
                host_port,
                container_port: p.private_port,
                proto,
                container: container.clone(),
                project: project.clone(),
                state: state.clone(),
                conflict: false,
            });
        }
    }
    out
}

/// `0.0.0.0` and `::` both mean "every interface" — treat them as no restriction.
fn normalise_ip(ip: &str) -> String {
    match ip {
        "" | "0.0.0.0" | "::" | "[::]" => String::new(),
        other => other.to_string(),
    }
}

/// Flag bindings that two different containers both claim.
///
/// Keyed on address as well as port: `127.0.0.1:8080` and `192.168.1.5:8080`
/// are different sockets and not in conflict, so keying on the port alone
/// would cry wolf. Two containers *can* legitimately appear here at once —
/// with `-a` a stopped container still reports the port it used to hold.
fn mark_conflicts(rows: &mut [Binding]) {
    let mut owners: HashMap<(String, u16, String), Vec<String>> = HashMap::new();
    for b in rows.iter() {
        let key = (b.host_ip.clone(), b.host_port, b.proto.clone());
        let names = owners.entry(key).or_default();
        if !names.contains(&b.container) {
            names.push(b.container.clone());
        }
    }
    for b in rows.iter_mut() {
        let key = (b.host_ip.clone(), b.host_port, b.proto.clone());
        b.conflict = owners.get(&key).is_some_and(|n| n.len() > 1);
    }
}

fn render(b: &Binding, show_bind: bool) -> Vec<String> {
    let (scol, sglyph) = theme::state_style(&b.state);

    // Colour carries the conflict, and the glyph carries it again for the
    // ascii theme, NO_COLOR, and anyone who cannot tell red from orange.
    let port = if b.conflict {
        format!("{} {}", c(&b.host_port.to_string(), p().red), c(g().fail, p().red))
    } else {
        cb(&b.host_port.to_string(), p().cyan)
    };

    let mut cells = vec![c(sglyph, scol), port];
    if show_bind {
        // An empty bind is the unrestricted common case; say so quietly rather
        // than leaving a hole in the column.
        cells.push(if b.host_ip.is_empty() { dim("all") } else { c(&b.host_ip, p().yellow) });
    }

    // tcp is the overwhelming default and reads as noise; udp is the surprise.
    let proto = if b.proto == "tcp" { dim(&b.proto) } else { c(&b.proto, p().magenta) };

    // A port that keeps its number through the mapping is the boring case.
    // A remap is the fact worth seeing, so only that one gets colour.
    let target = if b.container_port == b.host_port {
        dim(&b.container_port.to_string())
    } else {
        c(&b.container_port.to_string(), p().fg)
    };

    cells.extend([
        proto,
        cb(&b.container, if b.state == "running" { p().fg } else { p().gray }),
        if b.project.is_empty() { String::new() } else { c(&b.project, p().blue) },
        target,
    ]);
    cells
}

fn summary_line(rows: &[Binding]) -> String {
    let mut containers: Vec<&str> = rows.iter().map(|b| b.container.as_str()).collect();
    containers.sort_unstable();
    containers.dedup();

    let mut parts = vec![c(&plural(rows.len(), "port"), p().green)];
    parts.push(dim(&format!("across {}", plural(containers.len(), "container"))));

    let clashes = rows.iter().filter(|b| b.conflict).count();
    if clashes > 0 {
        parts.push(c(&format!("{clashes} in conflict"), p().red));
    }
    format!("\n{}", parts.join(&dim(" · ")))
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 { format!("{n} {word}") } else { format!("{n} {word}s") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bollard::models::{ContainerSummaryStateEnum, PortSummary, PortSummaryTypeEnum};
    use std::collections::HashMap;

    fn ct(name: &str, project: &str, ports: Vec<PortSummary>) -> ContainerSummary {
        ContainerSummary {
            id: Some(format!("{name}0000000000")),
            names: Some(vec![format!("/{name}")]),
            state: Some(ContainerSummaryStateEnum::RUNNING),
            ports: Some(ports),
            labels: Some(HashMap::from([
                (dk::COMPOSE_PROJECT.to_string(), project.to_string()),
                (dk::COMPOSE_SERVICE.to_string(), name.to_string()),
            ])),
            ..Default::default()
        }
    }

    fn port(ip: &str, public: Option<u16>, private: u16, udp: bool) -> PortSummary {
        PortSummary {
            ip: Some(ip.to_string()),
            private_port: private,
            public_port: public,
            typ: Some(if udp { PortSummaryTypeEnum::UDP } else { PortSummaryTypeEnum::TCP }),
        }
    }

    #[test]
    fn wildcard_addresses_are_not_restrictions() {
        for ip in ["", "0.0.0.0", "::", "[::]"] {
            assert_eq!(normalise_ip(ip), "");
        }
        assert_eq!(normalise_ip("127.0.0.1"), "127.0.0.1");
    }

    /// Docker reports a wildcard binding once for v4 and once for v6. A person
    /// published one port and should see one row.
    #[test]
    fn v4_and_v6_wildcards_collapse_to_one_row() {
        let list = vec![ct(
            "web",
            "shop",
            vec![port("0.0.0.0", Some(80), 80, false), port("::", Some(80), 80, false)],
        )];
        let rows = collect(&list);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].host_port, 80);
        assert!(rows[0].host_ip.is_empty());
    }

    /// The same port over tcp and udp is two real bindings, not a duplicate.
    #[test]
    fn tcp_and_udp_on_one_port_are_separate() {
        let list = vec![ct(
            "dns",
            "net",
            vec![port("0.0.0.0", Some(53), 53, false), port("0.0.0.0", Some(53), 53, true)],
        )];
        let mut rows = collect(&list);
        assert_eq!(rows.len(), 2);
        mark_conflicts(&mut rows);
        assert!(rows.iter().all(|b| !b.conflict), "different protocols do not clash");
    }

    #[test]
    fn exposed_but_unpublished_ports_are_skipped() {
        let list = vec![ct("api", "shop", vec![port("", None, 3000, false)])];
        assert!(collect(&list).is_empty());
    }

    #[test]
    fn two_containers_on_one_socket_conflict() {
        let list = vec![
            ct("old", "shop", vec![port("0.0.0.0", Some(8080), 80, false)]),
            ct("new", "shop", vec![port("0.0.0.0", Some(8080), 3000, false)]),
        ];
        let mut rows = collect(&list);
        mark_conflicts(&mut rows);
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|b| b.conflict));
    }

    /// Different addresses are different sockets, so flagging them would cry wolf.
    #[test]
    fn same_port_on_different_addresses_does_not_conflict() {
        let list = vec![
            ct("a", "shop", vec![port("127.0.0.1", Some(8080), 80, false)]),
            ct("b", "shop", vec![port("192.168.1.5", Some(8080), 80, false)]),
        ];
        let mut rows = collect(&list);
        mark_conflicts(&mut rows);
        assert!(rows.iter().all(|b| !b.conflict));
    }

    /// One container publishing the same port twice is a duplicate, not a clash.
    #[test]
    fn a_container_does_not_conflict_with_itself() {
        let list = vec![ct(
            "web",
            "shop",
            vec![port("0.0.0.0", Some(443), 443, false), port("::", Some(443), 443, false)],
        )];
        let mut rows = collect(&list);
        mark_conflicts(&mut rows);
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].conflict);
    }

    #[test]
    fn standalone_containers_carry_no_project() {
        let mut c = ct("box", "", vec![port("0.0.0.0", Some(9000), 9000, false)]);
        c.labels = None;
        let rows = collect(&[c]);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].project.is_empty());
        assert_eq!(rows[0].container, "box");
    }
}
