use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

use serde::Serialize;

#[derive(Clone)]
pub struct Wal {
    path: Option<PathBuf>,
}

#[derive(Serialize)]
struct Entry<'a, T: Serialize> {
    timestamp: String,
    operation: &'a str,
    params: T,
}

impl Wal {
    pub fn from_env() -> Self {
        let path = std::env::var("MEMQDRANT_WAL").ok().map(PathBuf::from).or_else(|| {
            dirs_home().map(|h| h.join(".memqdrant").join("wal.jsonl"))
        });
        if let Some(p) = &path {
            if let Some(parent) = p.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
        }
        Self { path }
    }

    pub fn log<T: Serialize>(&self, operation: &str, params: &T) {
        let Some(path) = &self.path else {
            return;
        };
        let entry = Entry {
            timestamp: now_rfc3339(),
            operation,
            params,
        };
        let line = match serde_json::to_string(&entry) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("wal serialize: {e}");
                return;
            }
        };
        if let Err(e) = append(path, &line) {
            tracing::warn!("wal append: {e}");
        }
    }
}

fn append(path: &PathBuf, line: &str) -> std::io::Result<()> {
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(f, "{line}")
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn now_rfc3339() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_rfc3339(secs)
}

// Minimal RFC3339 formatter. Kept private — we only need second precision for WAL.
fn format_rfc3339(mut secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    secs %= 86_400;
    let hour = (secs / 3600) as u32;
    let minute = ((secs / 60) % 60) as u32;
    let second = (secs % 60) as u32;

    // Civil-from-days, stolen from Howard Hinnant.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = (y + if m <= 2 { 1 } else { 0 }) as i64;
    format!("{year:04}-{m:02}-{d:02}T{hour:02}:{minute:02}:{second:02}Z")
}
