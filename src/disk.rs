//! `GET /system/df`, read so that every daemon version gives an answer.
//!
//! API 1.52 (Docker 29) changed the body. Older daemons send the pre-1.52
//! layout: `LayersSize` plus flat `Images`, `Containers`, `Volumes` and
//! `BuildCache` lists, and leave the totals to the client. Newer ones send
//! `ImageUsage`, `ContainerUsage`, `VolumeUsage` and `BuildCacheUsage` with
//! the totals worked out, and on an unversioned request send both.
//!
//! bollard only models the new layout, and its requests carry no version
//! prefix, so a daemon older than 29 answers in the old layout and bollard
//! decodes it into a struct of `None`s without complaint. That printed an
//! empty 0B table on Docker 26. So the body is fetched raw and read here, the
//! old layout is converted with the docker CLI's own arithmetic, and a body
//! in neither layout is an error rather than an empty table.

use anyhow::{Context, Result, bail};
use bollard::Docker;
use bollard::models::{
    BuildCacheDiskUsage, ContainersDiskUsage, ImagesDiskUsage, SystemDataUsageResponse,
    VolumesDiskUsage,
};
use futures_util::TryStreamExt;
use serde_json::{Map, Value};

/// Keys of the API 1.52+ layout.
const CURRENT: [&str; 4] = ["ImageUsage", "ContainerUsage", "VolumeUsage", "BuildCacheUsage"];
/// Keys of the layout every older daemon sends.
const LEGACY: [&str; 5] = ["LayersSize", "Images", "Containers", "Volumes", "BuildCache"];

pub async fn fetch(docker: &Docker) -> Result<SystemDataUsageResponse> {
    parse(&raw_df(docker).await?)
}

/// The undecoded `/system/df` body, over whatever transport `docker` uses.
///
/// bollard has no public raw GET, but its byte-stream calls hand back the body
/// untouched and its request modifier may rewrite the URI, which together give
/// one over the same unix, tcp, TLS, ssh or pipe connection as everything
/// else. The request goes out unversioned, as all of bollard's do, so the
/// daemon answers in its own layout, and `parse` reads either one.
async fn raw_df(docker: &Docker) -> Result<Vec<u8>> {
    let raw = docker.clone().with_request_modifier(|mut req| {
        let uri = req.uri();
        if let (Some(scheme), Some(authority)) = (uri.scheme_str(), uri.authority())
            && let Ok(df) = format!("{scheme}://{authority}/system/df").parse()
        {
            *req.uri_mut() = df;
        }
        req
    });
    raw.export_image("system-df")
        .try_fold(Vec::new(), |mut body, chunk| async move {
            body.extend_from_slice(&chunk);
            Ok(body)
        })
        .await
        .context("cannot read disk usage from the docker daemon")
}

pub fn parse(body: &[u8]) -> Result<SystemDataUsageResponse> {
    let value: Value =
        serde_json::from_slice(body).context("the daemon's disk usage is not valid JSON")?;
    let Some(obj) = value.as_object() else {
        bail!("the daemon's disk usage is not a JSON object");
    };
    if CURRENT.iter().any(|k| obj.contains_key(*k)) {
        return serde_json::from_value(value).context("cannot read the daemon's disk usage");
    }
    if LEGACY.iter().any(|k| obj.contains_key(*k)) {
        return Ok(legacy(obj));
    }
    let keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    bail!(
        "the daemon's disk usage is in a layout dok does not know (keys: {})",
        if keys.is_empty() { "none".to_string() } else { keys.join(", ") }
    )
}

/// The pre-1.52 layout, totalled the way `docker system df` totals it
/// (docker/cli `formatter/disk_usage.go`), so the two agree on old daemons.
fn legacy(obj: &Map<String, Value>) -> SystemDataUsageResponse {
    let list = |key: &str| -> Option<Vec<Value>> {
        obj.get(key).map(|v| v.as_array().cloned().unwrap_or_default())
    };

    let image_usage = list("Images").map(|images| {
        let layers = obj.get("LayersSize").and_then(Value::as_i64).unwrap_or(0);
        let used_by_containers: Vec<&Value> =
            images.iter().filter(|i| int(i, "Containers") > 0).collect();
        // Space held by images some container uses: each one's own layers,
        // less what it shares. -1 means the daemon did not compute it.
        let used: i64 = used_by_containers
            .iter()
            .filter(|i| int(i, "Size") != -1 && int(i, "SharedSize") != -1)
            .map(|i| int(i, "Size") - int(i, "SharedSize"))
            .sum();
        ImagesDiskUsage {
            active_count: Some(used_by_containers.len() as i64),
            total_count: Some(images.len() as i64),
            total_size: Some(layers),
            reclaimable: Some((layers - used).max(0)),
            items: Some(images),
        }
    });

    let container_usage = list("Containers").map(|containers| {
        let active = |c: &Value| {
            let state = c.get("State").and_then(Value::as_str).unwrap_or("");
            ["running", "paused", "restarting"].iter().any(|s| state.contains(s))
        };
        ContainersDiskUsage {
            active_count: Some(containers.iter().filter(|c| active(c)).count() as i64),
            total_count: Some(containers.len() as i64),
            total_size: Some(containers.iter().map(|c| int(c, "SizeRw")).sum()),
            reclaimable: Some(
                containers.iter().filter(|c| !active(c)).map(|c| int(c, "SizeRw")).sum(),
            ),
            items: Some(containers),
        }
    });

    let volume_usage = list("Volumes").map(|volumes| {
        // UsageData is null when the driver cannot report it; -1 is "unknown".
        let usage = |v: &Value| {
            let u = v.get("UsageData");
            let refs = u.map(|u| int(u, "RefCount")).unwrap_or(0);
            let size = u.and_then(|u| u.get("Size")).and_then(Value::as_i64).unwrap_or(-1);
            (refs, size)
        };
        let sized = || volumes.iter().map(usage).filter(|&(_, size)| size != -1);
        VolumesDiskUsage {
            active_count: Some(volumes.iter().filter(|v| usage(v).0 > 0).count() as i64),
            total_count: Some(volumes.len() as i64),
            total_size: Some(sized().map(|(_, size)| size).sum()),
            reclaimable: Some(sized().filter(|&(refs, _)| refs == 0).map(|(_, size)| size).sum()),
            items: Some(volumes),
        }
    });

    let build_cache_usage = list("BuildCache").map(|records| {
        let flag = |r: &Value, key| r.get(key).and_then(Value::as_bool).unwrap_or(false);
        // Shared records are counted under the images that hold them.
        let own = || records.iter().filter(|r| !flag(r, "Shared"));
        BuildCacheDiskUsage {
            active_count: Some(records.iter().filter(|r| flag(r, "InUse")).count() as i64),
            total_count: Some(records.len() as i64),
            total_size: Some(own().map(|r| int(r, "Size")).sum()),
            reclaimable: Some(own().filter(|r| !flag(r, "InUse")).map(|r| int(r, "Size")).sum()),
            items: Some(records),
        }
    });

    SystemDataUsageResponse { image_usage, container_usage, volume_usage, build_cache_usage }
}

fn int(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(Value::as_i64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real `/system/df` body from Docker 26.1.4 (API 1.45): two images,
    /// a running and an exited container, two volumes with `"Options": null`,
    /// `"ParentId": ""` on the images and an empty `BuildCache`. This is the
    /// body that used to render as an empty 0B table.
    const API_1_45: &str = include_str!("../tests/fixtures/system-df-api-1.45.json");

    fn counts(
        total: Option<i64>,
        reclaimable: Option<i64>,
        active: Option<i64>,
        count: Option<i64>,
    ) -> (i64, i64, i64, i64) {
        (total.unwrap(), reclaimable.unwrap(), active.unwrap(), count.unwrap())
    }

    #[test]
    fn a_docker_26_body_is_read_not_dropped() {
        let df = parse(API_1_45.as_bytes()).unwrap();

        let i = df.image_usage.unwrap();
        // LayersSize; both images have a container and share nothing.
        assert_eq!(
            counts(i.total_size, i.reclaimable, i.active_count, i.total_count),
            (12_927_959, 0, 2, 2)
        );
        assert_eq!(i.items.unwrap()[0]["RepoTags"][0], "alpine:3.20");

        let c = df.container_usage.unwrap();
        assert_eq!(
            counts(c.total_size, c.reclaimable, c.active_count, c.total_count),
            (0, 0, 1, 2)
        );
        assert_eq!(c.items.unwrap().len(), 2);

        let v = df.volume_usage.unwrap();
        assert_eq!(
            counts(v.total_size, v.reclaimable, v.active_count, v.total_count),
            (3_000_003, 0, 2, 2)
        );

        let b = df.build_cache_usage.unwrap();
        assert_eq!(
            counts(b.total_size, b.reclaimable, b.active_count, b.total_count),
            (0, 0, 0, 0)
        );
    }

    /// The arithmetic `docker system df` does on an old daemon, case by case.
    #[test]
    fn legacy_totals_match_the_docker_cli() {
        let body = r#"{
          "LayersSize": 1000,
          "Images": [
            {"Id": "a", "Containers": 1, "Size": 600, "SharedSize": 100},
            {"Id": "b", "Containers": 0, "Size": 300, "SharedSize": 100},
            {"Id": "c", "Containers": 2, "Size": -1, "SharedSize": -1}
          ],
          "Containers": [
            {"Id": "1", "State": "running", "SizeRw": 10},
            {"Id": "2", "State": "paused", "SizeRw": 20},
            {"Id": "3", "State": "exited", "SizeRw": 40},
            {"Id": "4", "State": "created"}
          ],
          "Volumes": [
            {"Name": "used", "UsageData": {"RefCount": 1, "Size": 5}},
            {"Name": "idle", "UsageData": {"RefCount": 0, "Size": 7}},
            {"Name": "unknown", "UsageData": {"RefCount": 0, "Size": -1}},
            {"Name": "nodata", "UsageData": null, "Options": null}
          ],
          "BuildCache": [
            {"ID": "x", "InUse": true, "Shared": false, "Size": 50},
            {"ID": "y", "InUse": false, "Shared": false, "Size": 30},
            {"ID": "z", "InUse": false, "Shared": true, "Size": 99}
          ]
        }"#;
        let df = parse(body.as_bytes()).unwrap();

        let i = df.image_usage.unwrap();
        // used = 600 - 100 from `a`; `c` is unknown and skipped, as docker does.
        assert_eq!(
            counts(i.total_size, i.reclaimable, i.active_count, i.total_count),
            (1000, 500, 2, 3)
        );

        let c = df.container_usage.unwrap();
        // running and paused are active; exited and created are reclaimable.
        assert_eq!(
            counts(c.total_size, c.reclaimable, c.active_count, c.total_count),
            (70, 40, 2, 4)
        );

        let v = df.volume_usage.unwrap();
        assert_eq!(
            counts(v.total_size, v.reclaimable, v.active_count, v.total_count),
            (12, 7, 1, 4)
        );

        let b = df.build_cache_usage.unwrap();
        assert_eq!(
            counts(b.total_size, b.reclaimable, b.active_count, b.total_count),
            (80, 30, 1, 3)
        );
    }

    #[test]
    fn the_current_layout_is_read_as_is() {
        let body = r#"{"ImageUsage": {"ActiveCount": 1, "TotalCount": 2, "Reclaimable": 3,
                       "TotalSize": 4, "Items": [{"Id": "a"}]}}"#;
        let df = parse(body.as_bytes()).unwrap();
        let i = df.image_usage.unwrap();
        assert_eq!(
            counts(i.total_size, i.reclaimable, i.active_count, i.total_count),
            (4, 3, 1, 2)
        );
        assert!(df.volume_usage.is_none());
    }

    /// Docker 29 answers an unversioned request with both layouts; the
    /// daemon's own totals win over recomputing them.
    #[test]
    fn when_both_layouts_arrive_the_current_one_wins() {
        let body = r#"{"LayersSize": 999, "Images": [],
                       "ImageUsage": {"TotalCount": 0, "TotalSize": 4}}"#;
        let df = parse(body.as_bytes()).unwrap();
        assert_eq!(df.image_usage.unwrap().total_size, Some(4));
    }

    /// The bug was a silent empty table; anything unrecognised must fail.
    #[test]
    fn an_unknown_body_is_an_error_not_an_empty_table() {
        let err = parse(br#"{"Something": 1}"#).unwrap_err().to_string();
        assert!(err.contains("keys: Something"), "{err}");
        assert!(parse(b"{}").is_err());
        assert!(parse(b"[]").is_err());
        assert!(parse(b"not json").is_err());
    }
}
