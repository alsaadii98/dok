//! `--json` — the same data the tables show, in a form a script can read.
//!
//! Three rules hold everywhere:
//!
//! 1. **Raw and derived travel together.** `{"size": 1181116006, "size_human":
//!    "1.1GB"}`. A script wants to compare the number; a person reading the
//!    JSON wants the string. Emitting only one loses somebody.
//! 2. **It is dok's data, not docker's.** The compose project, the service
//!    name, the reclaimable flag, the health verdict — the derived fields are
//!    the reason to use dok at all, so they are what the JSON carries. Run
//!    `docker inspect` if you want the raw API.
//! 3. **Key names are an interface.** Once released they do not change shape
//!    without a major version.

use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};

static JSON: AtomicBool = AtomicBool::new(false);

pub fn set(on: bool) {
    JSON.store(on, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    JSON.load(Ordering::Relaxed)
}

/// One document per command, pretty-printed because a person reads this as
/// often as a script does and `jq` does not care either way.
pub fn emit(v: &Value) {
    match serde_json::to_string_pretty(v) {
        Ok(s) => println!("{s}"),
        // stdout is already broken or the value is not representable; the
        // exit code still tells the caller, and there is nowhere to report to.
        Err(e) => eprintln!("cannot serialise output: {e}"),
    }
}

/// One object per line, for the commands that are streams rather than
/// snapshots. `dok logs -f --json | jq -c` should print as it goes, so this
/// never buffers.
pub fn emit_line(v: &Value) {
    use std::io::Write;
    if let Ok(s) = serde_json::to_string(v) {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{s}");
        let _ = out.flush();
    }
}

/// A byte count as both the number and what dok would have printed.
pub fn size(bytes: i64) -> Value {
    json!({ "bytes": bytes, "human": crate::fmt::bytes(bytes.max(0) as u64) })
}

/// A unix timestamp as the instant, the age in seconds, and dok's rendering.
pub fn age(created_unix: i64) -> Value {
    let secs = crate::dk::age_secs(created_unix);
    json!({ "created": created_unix, "age_seconds": secs, "age_human": crate::fmt::age(secs) })
}

/// `null` rather than `""` for a field that is genuinely absent — an empty
/// compose project is not a project named "".
pub fn opt(s: &str) -> Value {
    if s.is_empty() { Value::Null } else { json!(s) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Raw and derived travel together — a consumer that wants to compare
    /// sizes must never have to parse "1.1GB".
    #[test]
    fn size_carries_the_number_and_the_rendering() {
        let v = size(1_181_116_006);
        assert_eq!(v["bytes"], 1_181_116_006i64);
        assert!(v["human"].as_str().unwrap().ends_with("GB"));
    }

    /// A negative size is docker's way of saying unknown; it must not
    /// overflow into a nonsense unsigned value.
    #[test]
    fn a_negative_size_does_not_wrap() {
        let v = size(-1);
        assert_eq!(v["bytes"], -1i64);
        assert_eq!(v["human"], "0B");
    }

    #[test]
    fn age_carries_the_instant_the_seconds_and_the_rendering() {
        let now = chrono::Utc::now().timestamp();
        let v = age(now - 3600);
        assert_eq!(v["created"], now - 3600);
        assert!(v["age_seconds"].as_i64().unwrap() >= 3599);
        assert!(v["age_human"].is_string());
    }

    /// An absent compose project is null, not a project named "".
    #[test]
    fn absent_strings_are_null_not_empty() {
        assert!(opt("").is_null());
        assert_eq!(opt("demo-shop"), "demo-shop");
    }
}
