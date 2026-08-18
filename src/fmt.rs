//! Humanizers: byte sizes, durations, ANSI-aware widths, image name splitting.

/// Visible width of a string, ignoring ANSI SGR sequences.
pub fn visible_width(s: &str) -> usize {
    let mut w = 0usize;
    let mut chars = s.chars();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            // Skip through the terminating byte of the escape sequence.
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        w += char_width(ch);
    }
    w
}

/// Rough display width: enough for the glyphs we emit (ASCII, box drawing,
/// symbols, CJK, emoji) without pulling in a full unicode-width table.
fn char_width(c: char) -> usize {
    let u = c as u32;
    if u == 0 || (0x0300..=0x036F).contains(&u) || (0xFE00..=0xFE0F).contains(&u) {
        return 0; // combining marks / variation selectors
    }
    if (0x1100..=0x115F).contains(&u)
        || (0x2E80..=0xA4CF).contains(&u)
        || (0xAC00..=0xD7A3).contains(&u)
        || (0xF900..=0xFAFF).contains(&u)
        || (0xFF00..=0xFF60).contains(&u)
        || (0xFFE0..=0xFFE6).contains(&u)
        || (0x1F300..=0x1FAFF).contains(&u)
    {
        return 2;
    }
    1
}

/// Truncate to `max` visible columns, appending `…` when cut.
pub fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if visible_width(s) <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for ch in s.chars() {
        let cw = char_width(ch);
        if w + cw > max.saturating_sub(1) {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out.push('…');
    out
}

/// Pad to `width` visible columns.
pub fn pad(s: &str, width: usize) -> String {
    let w = visible_width(s);
    if w >= width { s.to_string() } else { format!("{s}{}", " ".repeat(width - w)) }
}

/// Right-align within `width` visible columns.
pub fn rpad(s: &str, width: usize) -> String {
    let w = visible_width(s);
    if w >= width { s.to_string() } else { format!("{}{s}", " ".repeat(width - w)) }
}

/// Docker-style decimal byte sizes: `1.2GB`, `340MB`, `12kB`.
pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 6] = ["B", "kB", "MB", "GB", "TB", "PB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1000.0 && i < UNITS.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    if i == 0 {
        format!("{n}B")
    } else if v < 10.0 {
        format!("{v:.1}{}", UNITS[i])
    } else {
        format!("{v:.0}{}", UNITS[i])
    }
}

/// Compact relative age: `12s`, `4m`, `3h`, `6d`, `5w`, `8mo`, `2y`.
pub fn age(secs: i64) -> String {
    let s = secs.max(0);
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m", s / 60),
        3600..=86399 => format!("{}h", s / 3600),
        86400..=604_799 => format!("{}d", s / 86400),
        604_800..=2_591_999 => format!("{}w", s / 604_800),
        2_592_000..=31_535_999 => format!("{}mo", s / 2_592_000),
        _ => format!("{}y", s / 31_536_000),
    }
}

/// Split `registry/namespace/name:tag` into (prefix, name, tag).
pub fn split_image(image: &str) -> (String, String, String) {
    // A digest reference keeps the whole `@sha256:…` as the "tag".
    let (repo, tag) = match image.split_once('@') {
        Some((r, d)) => (r, format!("@{}", &d[..d.len().min(14)])),
        None => match image.rsplit_once(':') {
            // A colon after the last slash is a tag; otherwise it is a port.
            Some((r, t)) if !t.contains('/') => (r, t.to_string()),
            _ => (image, "latest".to_string()),
        },
    };
    match repo.rsplit_once('/') {
        Some((prefix, name)) => (format!("{prefix}/"), name.to_string(), tag),
        None => (String::new(), repo.to_string(), tag),
    }
}

/// Short form of a 64-char container/image id.
pub fn short_id(id: &str) -> String {
    let id = id.strip_prefix("sha256:").unwrap_or(id);
    id.chars().take(12).collect()
}

/// Terminal width, defaulting to 120 when not a tty.
pub fn term_width() -> usize {
    terminal_size::terminal_size().map(|(terminal_size::Width(w), _)| w as usize).unwrap_or(120)
}

/// Drop the random task id from a swarm container name:
/// `api.1.hsnfrthap4x8v0xwy3nguwh85` -> `api.1`. The id is noise in a table —
/// the slot number is what tells two replicas apart.
pub fn short_task_name(name: &str) -> String {
    let parts: Vec<&str> = name.split('.').collect();
    if parts.len() >= 3
        && let (Some(id), Some(slot)) = (parts.last(), parts.get(parts.len() - 2))
        && id.len() >= 20
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && !slot.is_empty()
        && slot.chars().all(|c| c.is_ascii_digit())
    {
        return parts[..parts.len() - 1].join(".");
    }
    name.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swarm_task_ids_are_dropped() {
        assert_eq!(
            short_task_name("dokploy-postgres.1.c1btjeg93en04y5hw6ndqyn3c"),
            "dokploy-postgres.1"
        );
        assert_eq!(short_task_name("stack_api.12.abcdefghijklmnopqrstuvwxy"), "stack_api.12");
    }

    #[test]
    fn ordinary_names_are_left_alone() {
        assert_eq!(short_task_name("dokploy-traefik"), "dokploy-traefik");
        assert_eq!(short_task_name("shop-api-1"), "shop-api-1");
        // A dotted name whose last part is short is not a task id.
        assert_eq!(short_task_name("api.1.beta"), "api.1.beta");
        // Nor is one whose slot is not a number.
        assert_eq!(
            short_task_name("api.x.hsnfrthap4x8v0xwy3nguwh85"),
            "api.x.hsnfrthap4x8v0xwy3nguwh85"
        );
    }
}
