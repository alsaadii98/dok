//! Canned data for `--demo`.
//!
//! Lets the docs (and anyone without a daemon) see real rendering of a
//! plausible stack, without exposing whatever happens to be running on the
//! machine that generated the screenshots.

use bollard::models::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

static DEMO: AtomicBool = AtomicBool::new(false);

pub fn set(on: bool) {
    DEMO.store(on, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    DEMO.load(Ordering::Relaxed)
}

const DAY: i64 = 86400;

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn labels(project: &str, service: &str) -> HashMap<String, String> {
    HashMap::from([
        (crate::dk::COMPOSE_PROJECT.to_string(), project.to_string()),
        (crate::dk::COMPOSE_SERVICE.to_string(), service.to_string()),
        ("com.docker.compose.container-number".to_string(), "1".to_string()),
        (
            "org.opencontainers.image.source".to_string(),
            format!("https://github.com/demo-shop/{service}"),
        ),
    ])
}

fn port(public: u16, private: u16) -> PortSummary {
    PortSummary {
        ip: Some("0.0.0.0".into()),
        private_port: private,
        public_port: Some(public),
        typ: Some(PortSummaryTypeEnum::TCP),
    }
}

fn endpoint(net: &str, ip: &str) -> ContainerSummaryNetworkSettings {
    ContainerSummaryNetworkSettings {
        networks: Some(HashMap::from([(
            net.to_string(),
            EndpointSettings {
                ip_address: Some(ip.into()),
                network_id: Some("f3a1c0de1234".into()),
                ..Default::default()
            },
        )])),
    }
}

fn mount(name: &str, dest: &str) -> MountPoint {
    MountPoint {
        typ: Some("volume".into()),
        name: Some(name.into()),
        source: Some(format!("/var/lib/docker/volumes/{name}/_data")),
        destination: Some(dest.into()),
        driver: Some("local".into()),
        rw: Some(true),
        ..Default::default()
    }
}

#[allow(clippy::too_many_arguments)]
fn container(
    id: &str,
    name: &str,
    image: &str,
    state: ContainerSummaryStateEnum,
    status: &str,
    created_ago: i64,
    ports: Vec<PortSummary>,
    project: Option<(&str, &str)>,
    health: Option<ContainerSummaryHealthStatusEnum>,
    ip: Option<&str>,
    mounts: Vec<MountPoint>,
) -> ContainerSummary {
    ContainerSummary {
        id: Some(id.into()),
        names: Some(vec![format!("/{name}")]),
        image: Some(image.into()),
        image_id: Some(format!("sha256:{id}0f2b7c4d")),
        command: Some("docker-entrypoint.sh".into()),
        created: Some(now() - created_ago),
        ports: Some(ports),
        labels: project.map(|(p, s)| labels(p, s)),
        state: Some(state),
        status: Some(status.into()),
        network_settings: ip.map(|ip| endpoint("demo-shop_default", ip)),
        mounts: Some(mounts),
        health: health.map(|h| ContainerSummaryHealth {
            status: Some(h),
            failing_streak: Some(if h == ContainerSummaryHealthStatusEnum::UNHEALTHY {
                3
            } else {
                0
            }),
        }),
        ..Default::default()
    }
}

pub fn containers() -> Vec<ContainerSummary> {
    use ContainerSummaryHealthStatusEnum as H;
    use ContainerSummaryStateEnum as S;
    vec![
        container(
            "a1b2c3d4e5f6",
            "demo-shop-api-1",
            "ghcr.io/demo-shop/api:1.4.2",
            S::RUNNING,
            "Up 3 hours (healthy)",
            6 * DAY,
            vec![port(8080, 3000)],
            Some(("demo-shop", "api")),
            Some(H::HEALTHY),
            Some("172.19.0.4"),
            vec![],
        ),
        container(
            "b2c3d4e5f6a1",
            "demo-shop-web-1",
            "nginx:1.27-alpine",
            S::RUNNING,
            "Up 3 hours",
            6 * DAY,
            vec![port(80, 80), port(443, 443)],
            Some(("demo-shop", "web")),
            None,
            Some("172.19.0.5"),
            vec![],
        ),
        container(
            "c3d4e5f6a1b2",
            "demo-shop-postgres-1",
            "postgres:16-alpine",
            S::RUNNING,
            "Up 3 hours (healthy)",
            6 * DAY,
            vec![port(5432, 5432)],
            Some(("demo-shop", "postgres")),
            Some(H::HEALTHY),
            Some("172.19.0.2"),
            vec![mount("demo-shop_pgdata", "/var/lib/postgresql/data")],
        ),
        container(
            "d4e5f6a1b2c3",
            "demo-shop-redis-1",
            "redis:7-alpine",
            S::RUNNING,
            "Up 3 hours",
            6 * DAY,
            vec![port(6379, 6379)],
            Some(("demo-shop", "redis")),
            None,
            Some("172.19.0.3"),
            vec![mount("demo-shop_cache", "/data")],
        ),
        container(
            "e5f6a1b2c3d4",
            "demo-shop-worker-1",
            "ghcr.io/demo-shop/worker:1.4.2",
            S::EXITED,
            "Exited (137) 2 days ago",
            6 * DAY,
            vec![],
            Some(("demo-shop", "worker")),
            None,
            None,
            vec![],
        ),
        container(
            "f6a1b2c3d4e5",
            "metrics-1",
            "grafana/grafana:11.2.0",
            S::RUNNING,
            "Up 8 hours (unhealthy)",
            21 * DAY,
            vec![port(3001, 3000)],
            Some(("observability", "grafana")),
            Some(H::UNHEALTHY),
            Some("172.20.0.2"),
            vec![],
        ),
        container(
            "0a9b8c7d6e5f",
            "scratch-box",
            "alpine:3.20",
            S::CREATED,
            "Created",
            40 * DAY,
            vec![],
            None,
            None,
            None,
            vec![],
        ),
    ]
}

fn image(
    id: &str,
    tags: &[&str],
    size: i64,
    shared: i64,
    created_ago: i64,
    used: i64,
) -> ImageSummary {
    ImageSummary {
        id: format!("sha256:{id}"),
        parent_id: String::new(),
        repo_tags: tags.iter().map(|t| t.to_string()).collect(),
        repo_digests: vec![],
        created: now() - created_ago,
        size,
        shared_size: shared,
        labels: HashMap::new(),
        containers: used,
        manifests: None,
        descriptor: None,
    }
}

pub fn images() -> Vec<ImageSummary> {
    vec![
        image(
            "9f2c1a7b3e40",
            &["ghcr.io/demo-shop/worker:1.4.2"],
            1_140_000_000,
            142_000_000,
            2 * DAY,
            1,
        ),
        image("57c72fd2a128", &["postgres:16-alpine"], 411_000_000, 0, 32 * DAY, 1),
        image(
            "2372ac0330be",
            &["ghcr.io/demo-shop/api:1.4.2", "ghcr.io/demo-shop/api:latest"],
            271_000_000,
            142_000_000,
            2 * DAY,
            1,
        ),
        image("bb14a2d9f7c1", &["grafana/grafana:11.2.0"], 168_000_000, 0, 90 * DAY, 1),
        image("31f0a8c4b9de", &["nginx:1.27-alpine"], 78_000_000, 0, 12 * DAY, 1),
        image("e7723ff73d96", &["redis:7-alpine"], 59_000_000, 0, 21 * DAY, 1),
        image("7d1c4b8a2f30", &["alpine:3.20"], 8_100_000, 0, 60 * DAY, 1),
        image("4c9e2f81a76b", &[], 244_000_000, 0, 9 * DAY, 0),
    ]
}

pub fn networks() -> Vec<Network> {
    let net = |name: &str, driver: &str, subnet: Option<&str>| Network {
        name: Some(name.into()),
        id: Some("f3a1c0de1234".into()),
        driver: Some(driver.into()),
        scope: Some("local".into()),
        ipam: subnet.map(|s| Ipam {
            driver: Some("default".into()),
            config: Some(vec![IpamConfig { subnet: Some(s.into()), ..Default::default() }]),
            options: None,
        }),
        ..Default::default()
    };
    vec![
        net("bridge", "bridge", Some("172.17.0.0/16")),
        net("demo-shop_default", "bridge", Some("172.19.0.0/16")),
        net("observability_default", "bridge", Some("172.20.0.0/16")),
        net("host", "host", None),
        net("none", "null", None),
    ]
}

pub fn volumes() -> Vec<Volume> {
    let vol = |name: &str, size: i64, refs: i64| Volume {
        name: name.into(),
        driver: "local".into(),
        mountpoint: format!("/var/lib/docker/volumes/{name}/_data"),
        labels: HashMap::new(),
        options: HashMap::new(),
        usage_data: Some(VolumeUsageData { size, ref_count: refs }),
        ..Default::default()
    };
    vec![
        vol("demo-shop_pgdata", 312_000_000, 1),
        vol("demo-shop_cache", 3_100_000, 1),
        vol("old-release_uploads", 128_000_000, 0),
    ]
}

pub fn df() -> SystemDataUsageResponse {
    let items = |v: Vec<serde_json::Value>| Some(v);
    SystemDataUsageResponse {
        image_usage: Some(ImagesDiskUsage {
            active_count: Some(7),
            total_count: Some(8),
            reclaimable: Some(244_000_000),
            total_size: Some(2_379_100_000),
            items: items(
                images()
                    .iter()
                    .map(|i| {
                        serde_json::json!({
                            "Id": i.id,
                            "RepoTags": i.repo_tags,
                            "Size": i.size,
                            "Containers": i.containers,
                        })
                    })
                    .collect(),
            ),
        }),
        container_usage: Some(ContainersDiskUsage {
            active_count: Some(5),
            total_count: Some(7),
            reclaimable: Some(96_000),
            total_size: Some(141_000),
            items: items(
                containers()
                    .iter()
                    .enumerate()
                    .map(|(n, c)| {
                        serde_json::json!({
                            "Id": c.id,
                            "Names": c.names,
                            "State": c.state.map(|s| s.to_string()),
                            "SizeRw": 12_000 + (n as i64) * 9_000,
                        })
                    })
                    .collect(),
            ),
        }),
        volume_usage: Some(VolumesDiskUsage {
            active_count: Some(2),
            total_count: Some(3),
            reclaimable: Some(128_000_000),
            total_size: Some(443_100_000),
            items: items(
                volumes()
                    .iter()
                    .map(|v| {
                        serde_json::json!({
                            "Name": v.name,
                            "UsageData": {
                                "Size": v.usage_data.as_ref().map(|u| u.size),
                                "RefCount": v.usage_data.as_ref().map(|u| u.ref_count),
                            },
                        })
                    })
                    .collect(),
            ),
        }),
        build_cache_usage: Some(BuildCacheDiskUsage {
            active_count: Some(2),
            total_count: Some(14),
            reclaimable: Some(869_000_000),
            total_size: Some(892_000_000),
            items: items(vec![
                serde_json::json!({"ID":"k2j9x1","Description":"pulled from docker.io/library/node:20-alpine","Size":142_000_000,"InUse":false,"UsageCount":3}),
                serde_json::json!({"ID":"a7f2b0","Description":"mount / from exec /bin/sh -c npm ci --omit=dev","Size":318_000_000,"InUse":false,"UsageCount":2}),
                serde_json::json!({"ID":"c1d8e4","Description":"mount / from exec /bin/sh -c cargo build --release","Size":409_000_000,"InUse":true,"UsageCount":7}),
            ]),
        }),
    }
}

/// A fabricated `docker inspect` for the demo api container.
pub fn inspect(_name: &str) -> ContainerInspectResponse {
    ContainerInspectResponse {
        id: Some("a1b2c3d4e5f60f2b7c4d8e9a0b1c2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f7a8b9c".into()),
        name: Some("/demo-shop-api-1".into()),
        image: Some(
            "sha256:2372ac0330be1f8c5d92e4a7b3c1d0e9f8a7b6c5d4e3f2a1b0c9d8e7f6a5b4c3".into(),
        ),
        platform: Some("linux".into()),
        restart_count: Some(2),
        size_rw: Some(48_000),
        state: Some(ContainerState {
            status: Some(ContainerStateStatusEnum::RUNNING),
            running: Some(true),
            pid: Some(21874),
            started_at: Some("2026-08-17T06:12:44.118Z".into()),
            exit_code: Some(0),
            health: Some(Health {
                status: Some(HealthStatusEnum::HEALTHY),
                failing_streak: Some(0),
                log: Some(vec![HealthcheckResult {
                    exit_code: Some(0),
                    output: Some("{\"status\":\"ok\",\"db\":\"up\",\"queue\":\"up\"}\n".into()),
                    ..Default::default()
                }]),
            }),
            ..Default::default()
        }),
        config: Some(ContainerConfig {
            image: Some("ghcr.io/demo-shop/api:1.4.2".into()),
            entrypoint: Some(vec!["docker-entrypoint.sh".into()]),
            cmd: Some(vec!["node".into(), "dist/server.js".into()]),
            working_dir: Some("/srv/app".into()),
            user: Some("node".into()),
            stop_signal: Some("SIGTERM".into()),
            env: Some(vec![
                "NODE_ENV=production".into(),
                "PORT=3000".into(),
                "DATABASE_URL=postgres://api@postgres:5432/shop".into(),
                "REDIS_URL=redis://redis:6379/0".into(),
                "JWT_SECRET=s3cr3t-do-not-print".into(),
                "STRIPE_API_KEY=sk_live_51H8xQexample".into(),
                "LOG_LEVEL=info".into(),
            ]),
            healthcheck: Some(HealthConfig {
                test: Some(vec!["CMD-SHELL".into(), "curl -fsS localhost:3000/healthz".into()]),
                interval: Some(30_000_000_000),
                timeout: Some(5_000_000_000),
                retries: Some(3),
                ..Default::default()
            }),
            labels: Some(labels("demo-shop", "api")),
            ..Default::default()
        }),
        host_config: Some(HostConfig {
            memory: Some(536_870_912),
            nano_cpus: Some(1_500_000_000),
            pids_limit: Some(512),
            network_mode: Some("demo-shop_default".into()),
            readonly_rootfs: Some(true),
            cap_drop: Some(vec!["ALL".into()]),
            restart_policy: Some(RestartPolicy {
                name: Some(RestartPolicyNameEnum::UNLESS_STOPPED),
                maximum_retry_count: Some(0),
            }),
            log_config: Some(HostConfigLogConfig { typ: Some("json-file".into()), config: None }),
            ..Default::default()
        }),
        network_settings: Some(NetworkSettings {
            ports: Some(HashMap::from([(
                "3000/tcp".to_string(),
                Some(vec![PortBinding {
                    host_ip: Some("0.0.0.0".into()),
                    host_port: Some("8080".into()),
                }]),
            )])),
            networks: Some(HashMap::from([(
                "demo-shop_default".to_string(),
                EndpointSettings {
                    ip_address: Some("172.19.0.4".into()),
                    aliases: Some(vec!["api".into()]),
                    ..Default::default()
                },
            )])),
            ..Default::default()
        }),
        mounts: Some(vec![
            mount("demo-shop_uploads", "/srv/app/uploads"),
            MountPoint {
                typ: Some("bind".into()),
                source: Some("/etc/demo-shop/api.toml".into()),
                destination: Some("/srv/app/config.toml".into()),
                rw: Some(false),
                ..Default::default()
            },
        ]),
        ..Default::default()
    }
}

/// `ps` output as the daemon would return it, for `dok top --demo`.
pub fn top(name: &str) -> ContainerTopResponse {
    let titles = ["PID", "PPID", "USER", "%CPU", "%MEM", "ELAPSED", "COMMAND"]
        .iter()
        .map(|t| t.to_string())
        .collect();
    let row = |p: &str, pp: &str, u: &str, c: &str, m: &str, cmd: &str| {
        vec![p, pp, u, c, m, "03:11:47", cmd].into_iter().map(String::from).collect::<Vec<_>>()
    };
    let processes = match name {
        n if n.contains("postgres") => vec![
            row("667", "620", "70", "0.0", "0.3", "postgres"),
            row("759", "667", "70", "0.0", "0.1", "postgres: checkpointer"),
            row("760", "667", "70", "0.0", "0.1", "postgres: background writer"),
            row("762", "667", "70", "0.1", "0.2", "postgres: walwriter"),
            row("764", "667", "70", "0.0", "0.1", "postgres: autovacuum launcher"),
        ],
        n if n.contains("redis") => {
            vec![row("666", "619", "999", "1.4", "0.1", "redis-server *:6379")]
        }
        n if n.contains("web") => vec![
            row("701", "688", "root", "0.0", "0.1", "nginx: master process nginx -g daemon off;"),
            row("742", "701", "nginx", "0.2", "0.1", "nginx: worker process"),
            row("743", "701", "nginx", "0.1", "0.1", "nginx: worker process"),
        ],
        _ => vec![
            row("21874", "21850", "node", "2.1", "1.8", "node dist/server.js"),
            row("21902", "21874", "node", "0.4", "0.6", "node dist/worker.js --queue=email"),
        ],
    };
    ContainerTopResponse { titles: Some(titles), processes: Some(processes) }
}

/// Interleaved log lines, already in docker's `<rfc3339> <message>` shape.
pub fn logs() -> Vec<(&'static str, &'static str, bool)> {
    vec![
        (
            "demo-shop-api-1",
            "{\"level\":\"info\",\"msg\":\"listening\",\"port\":3000,\"env\":\"production\"}",
            false,
        ),
        ("demo-shop-postgres-1", "LOG:  database system is ready to accept connections", true),
        ("demo-shop-redis-1", "1:M * Ready to accept connections tcp", false),
        ("demo-shop-web-1", "172.19.0.1 - GET /api/products HTTP/1.1 200 1420 12ms", false),
        (
            "demo-shop-api-1",
            "{\"level\":\"info\",\"msg\":\"cart.checkout\",\"user\":\"u_8812\",\"items\":3,\"total\":\"49.90\"}",
            false,
        ),
        ("demo-shop-web-1", "172.19.0.1 - POST /api/checkout HTTP/1.1 402 88 240ms", false),
        (
            "demo-shop-api-1",
            "{\"level\":\"warn\",\"msg\":\"payment declined\",\"gateway\":\"stripe\",\"code\":\"card_declined\"}",
            false,
        ),
        (
            "demo-shop-postgres-1",
            "ERROR:  duplicate key value violates unique constraint \"orders_ref_key\"",
            true,
        ),
        (
            "demo-shop-api-1",
            "{\"level\":\"error\",\"msg\":\"order insert failed\",\"retry\":true,\"attempt\":2}",
            false,
        ),
        ("demo-shop-redis-1", "1:M * Background saving terminated with success", false),
    ]
}

/// A believable minute of daemon events, newest last, for `dok events --demo`.
pub fn events() -> Vec<EventMessage> {
    let t0 = now() - 15 * 60;
    let ev = |offset: i64,
              typ: EventMessageTypeEnum,
              action: &str,
              id: &str,
              attrs: &[(&str, &str)]| EventMessage {
        typ: Some(typ),
        action: Some(action.to_string()),
        actor: Some(EventActor {
            id: Some(id.to_string()),
            attributes: Some(attrs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()),
        }),
        scope: Some(EventMessageScopeEnum::LOCAL),
        time: Some(t0 + offset),
        time_nano: Some((t0 + offset) * 1_000_000_000),
    };
    let c = EventMessageTypeEnum::CONTAINER;
    let img = EventMessageTypeEnum::IMAGE;
    let net = EventMessageTypeEnum::NETWORK;
    let vol = EventMessageTypeEnum::VOLUME;

    vec![
        ev(0, img, "pull", "ghcr.io/demo/api:1.9.2", &[("name", "ghcr.io/demo/api:1.9.2")]),
        ev(3, net, "create", "5a4f9c1e7b20", &[("name", "demo-shop_default"), ("type", "bridge")]),
        ev(5, vol, "create", "demo-shop_pgdata", &[("driver", "local")]),
        ev(
            9,
            c,
            "create",
            "9c1d0a41f8b2",
            &[("name", "demo-shop-postgres-1"), ("image", "postgres:16-alpine")],
        ),
        ev(
            10,
            c,
            "start",
            "9c1d0a41f8b2",
            &[("name", "demo-shop-postgres-1"), ("image", "postgres:16-alpine")],
        ),
        ev(
            14,
            c,
            "create",
            "3f6b8d02ac57",
            &[("name", "demo-shop-api-1"), ("image", "ghcr.io/demo/api:1.9.2")],
        ),
        ev(
            15,
            c,
            "start",
            "3f6b8d02ac57",
            &[("name", "demo-shop-api-1"), ("image", "ghcr.io/demo/api:1.9.2")],
        ),
        ev(31, c, "health_status: healthy", "9c1d0a41f8b2", &[("name", "demo-shop-postgres-1")]),
        ev(48, c, "health_status: unhealthy", "7d2e5b90c114", &[("name", "demo-shop-worker-1")]),
        ev(
            52,
            c,
            "die",
            "7d2e5b90c114",
            &[
                ("name", "demo-shop-worker-1"),
                ("image", "ghcr.io/demo/worker:1.9.2"),
                ("exitCode", "137"),
            ],
        ),
        ev(53, c, "kill", "7d2e5b90c114", &[("name", "demo-shop-worker-1"), ("signal", "9")]),
        ev(
            55,
            c,
            "restart",
            "7d2e5b90c114",
            &[("name", "demo-shop-worker-1"), ("image", "ghcr.io/demo/worker:1.9.2")],
        ),
        ev(61, img, "untag", "ghcr.io/demo/api:1.9.1", &[("name", "ghcr.io/demo/api:1.9.1")]),
    ]
}
