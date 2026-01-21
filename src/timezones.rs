/////////
/// Detecting and managing timezones.
////////
use anyhow::Result;
use std::fs;
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;

// Loads sorted timezones from system
pub fn load_timezones() -> Result<Vec<String>> {
    let candidates = [
        "/usr/share/zoneinfo/zone1970.tab", // fallback
        "/usr/share/zoneinfo/zone.tab",     // Standard
    ];

    for path in candidates {
        if let Ok(content) = fs::read_to_string(path) {
            let mut zones = Vec::new();
            for line in content.lines() {
                let line = line.trim();
                // Skip empty lines and comments.
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let mut parts = line.split('\t');
                let _cc = parts.next();
                let _coords = parts.next();
                let name = parts.next(); // Timezone name
                if let Some(name) = name {
                    zones.push(name.to_string());
                }
            }
            zones.sort();
            zones.dedup(); // Remove duplicates.

            // Ensure "UTC" is always an option
            if !zones
                .iter()
                .any(|zone| matches!(zone.as_str(), "UTC" | "Etc/UTC" | "Etc/GMT" | "GMT"))
            {
                zones.push("UTC".to_string());
                zones.sort();
            }

            if !zones.is_empty() {
                return Ok(zones);
            }
        }
    }

    Err(anyhow::anyhow!("No timezone list found"))
}

pub fn find_timezone_index(zones: &[String], value: &str) -> Option<usize> {
    zones.iter().position(|zone| zone == value)
}

// Debug messages to a log file
fn log_debug(message: &str) {
    let _ = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/run/nebula/timezone-detect.log")
        .and_then(|mut file| {
            use std::io::Write;
            writeln!(file, "{}", message)
        });
}

// Normalizes timezone
fn normalize_timezone(zones: &[String], tz: &str) -> Option<String> {
    if zones.iter().any(|zone| zone == tz) {
        return Some(tz.to_string());
    }

    // Check for common aliases if the direct match fails
    let candidates = match tz {
        "UTC" | "Etc/UTC" | "Etc/GMT" | "GMT" => ["UTC", "Etc/UTC", "Etc/GMT", "GMT"],
        _ => [tz, "Etc/UTC", "UTC", "Etc/GMT"], // Prioritize original, then UTC variants
    };

    for candidate in candidates {
        if zones.iter().any(|zone| zone == candidate) {
            return Some(candidate.to_string());
        }
    }

    None
}

// Checks if a given timezone string represents a UTC variant.
fn is_utc_variant(tz: &str) -> bool {
    matches!(tz, "UTC" | "Etc/UTC" | "Etc/GMT" | "GMT")
}

// A JSON parser to extract a string field value from a JSON
fn json_string_field(body: &str, key: &str) -> Option<String> {
    let needle = format!("\"{}\"", key);
    let start = body.find(&needle)?;
    let after_key = &body[start + needle.len()..];
    let colon = after_key.find(':')?;
    let after_colon = &after_key[colon + 1..].trim_start();
    let quote = after_colon.find('"')?;
    let rest = &after_colon[quote + 1..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

// Useses the `ipapi.co` to detect the user's timezone based on their IP address
pub fn detect_timezone_geoip(zones: &[String]) -> Option<String> {
    // Skip GeoIP detection in offline and skip network mode
    if std::env::var("NEBULA_SKIP_NETWORK").ok().as_deref() == Some("1")
        || std::env::var("NEBULA_OFFLINE_ONLY").ok().as_deref() == Some("1")
    {
        log_debug("detect_timezone: geoip skipped (offline)");
        return None;
    }

    // Retry logic for the curl request
    for attempt in 1..=5 {
        let output = Command::new("curl")
            .args([
                "-fsS",
                "--connect-timeout",
                "2", // Timeout for connection
                "--max-time",
                "4", // Max time
                "https://ipapi.co/json/",
            ])
            .output();
        match output {
            Ok(output) if output.status.success() => {
                let body = String::from_utf8_lossy(&output.stdout);
                let tz = json_string_field(&body, "timezone");
                if let Some(tz) = tz {
                    log_debug(&format!("detect_timezone: geoip timezone {}", tz));
                    if let Some(value) = normalize_timezone(zones, &tz) {
                        return Some(value);
                    }
                }
                log_debug("detect_timezone: geoip did not match list");
                return None;
            }
            _ => {
                log_debug(&format!(
                    "detect_timezone: geoip curl failed (attempt {})",
                    attempt
                ));
                sleep(Duration::from_millis(700)); // Wait before retrying
            }
        }
    }
    None // All GeoIP attempts failed
}

// Detect the local timezone from system files like `/etc/timezone` or `/etc/localtime`
// === We should remove this in future === //
pub fn detect_timezone_local(zones: &[String]) -> Option<String> {
    log_debug("detect_timezone: local start");
    log_debug(&format!("detect_timezone: zones={}", zones.len()));

    // Try reading from `/etc/timezone`
    if let Ok(content) = fs::read_to_string("/etc/timezone") {
        log_debug("detect_timezone: /etc/timezone found");
        if let Some(line) = content
            .lines()
            .map(|line| line.trim())
            .find(|line| !line.is_empty())
        {
            log_debug(&format!("detect_timezone: /etc/timezone line={}", line));
            if let Some(value) = normalize_timezone(zones, line) {
                // Prefer non-UTC timezones from /etc/timezone
                if !is_utc_variant(&value) {
                    log_debug(&format!("detect_timezone: using /etc/timezone {}", value));
                    return Some(value);
                }
                log_debug("detect_timezone: /etc/timezone is UTC, deferring");
            }
        }
    }

    // Try reading from `/etc/localtime` symlink
    let tz_path = fs::read_link("/etc/localtime")
        .ok()
        .or_else(|| fs::canonicalize("/etc/localtime").ok());
    if let Some(path) = tz_path {
        log_debug(&format!(
            "detect_timezone: /etc/localtime -> {}",
            path.display()
        ));
        // Attempt to strip the /usr/share/zoneinfo/ prefix to get the timezone name
        if let Ok(stripped) = path.strip_prefix("/usr/share/zoneinfo/") {
            if let Some(tz) = stripped.to_str() {
                log_debug(&format!("detect_timezone: localtime stripped {}", tz));
                if let Some(value) = normalize_timezone(zones, tz) {
                    if !is_utc_variant(&value) {
                        log_debug(&format!("detect_timezone: using /etc/localtime {}", value));
                        return Some(value);
                    }
                    log_debug("detect_timezone: /etc/localtime is UTC, deferring");
                }
            }
        // Fallback
        } else if let Some(tz) = path.to_str().and_then(|p| {
            p.split("/usr/share/zoneinfo/")
                .nth(1)
                .map(|suffix| suffix.to_string())
        }) {
            log_debug(&format!("detect_timezone: localtime suffix {}", tz));
            if let Some(value) = normalize_timezone(zones, &tz) {
                if !is_utc_variant(&value) {
                    log_debug(&format!("detect_timezone: using /etc/localtime {}", value));
                    return Some(value);
                }
                log_debug("detect_timezone: /etc/localtime is UTC, deferring");
            }
        }
    } else {
        log_debug("detect_timezone: /etc/localtime not found");
    }

    log_debug("detect_timezone: failed");
    None
}
