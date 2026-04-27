use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::util::now_rfc3339;

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
        let path = std::env::var("PALAZZO_WAL")
            .ok()
            .map(PathBuf::from)
            .or_else(|| dirs_home().map(|h| h.join(".palazzo").join("wal.jsonl")));
        if let Some(p) = &path
            && let Some(parent) = p.parent()
        {
            let _ = std::fs::create_dir_all(parent);
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

fn append(path: &Path, line: &str) -> std::io::Result<()> {
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(f, "{line}")
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}
