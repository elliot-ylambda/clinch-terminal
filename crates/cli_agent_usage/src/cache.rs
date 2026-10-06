//! Incremental file cache: parse a file only when its (mtime, size) changed.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

struct Entry<T> {
    mtime: SystemTime,
    size: u64,
    value: T,
}

pub struct ScanCache<T> {
    entries: HashMap<PathBuf, Entry<T>>,
}

impl<T> ScanCache<T> {
    pub fn new() -> Self {
        ScanCache {
            entries: HashMap::new(),
        }
    }

    pub fn get_or_parse(
        &mut self,
        path: &Path,
        mtime: SystemTime,
        size: u64,
        parse: impl FnOnce(&Path) -> T,
    ) -> &T {
        let fresh = match self.entries.get(path) {
            Some(e) => e.mtime == mtime && e.size == size,
            None => false,
        };
        if !fresh {
            let value = parse(path);
            self.entries
                .insert(path.to_path_buf(), Entry { mtime, size, value });
        }
        &self.entries.get(path).expect("just inserted").value
    }

    /// Lists current sources and releases cached values for files removed since
    /// the previous scan. An incomplete walk cannot prove a file was removed,
    /// so transient permission and I/O errors leave the cache intact.
    pub(crate) fn scan_dir(&mut self, root: &Path, ext: &str) -> Vec<(PathBuf, SystemTime, u64)> {
        let scan = scan_dir_with_status(root, ext);
        self.retain_scanned_files(&scan);
        scan.files
    }

    fn retain_scanned_files(&mut self, scan: &DirectoryScan) {
        if !scan.complete {
            return;
        }
        let paths: HashSet<&Path> = scan
            .files
            .iter()
            .map(|(path, _, _)| path.as_path())
            .collect();
        let previous_len = self.entries.len();
        self.entries
            .retain(|path, _| paths.contains(path.as_path()));
        if self.entries.len() < previous_len / 2 {
            self.entries.shrink_to_fit();
        }
    }
}

impl<T> Default for ScanCache<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// Recursively list files under `root` whose name ends with `ext` (e.g. ".jsonl").
/// Missing/unreadable dir → empty vec (fail-soft).
pub fn scan_dir(root: &Path, ext: &str) -> Vec<(PathBuf, SystemTime, u64)> {
    scan_dir_with_status(root, ext).files
}

struct DirectoryScan {
    files: Vec<(PathBuf, SystemTime, u64)>,
    complete: bool,
}

fn scan_dir_with_status(root: &Path, ext: &str) -> DirectoryScan {
    let mut scan = DirectoryScan {
        files: Vec::new(),
        complete: true,
    };
    for entry in walkdir::WalkDir::new(root) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                scan.complete = false;
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if !path.to_string_lossy().ends_with(ext) {
            continue;
        }
        if let Ok(md) = entry.metadata() {
            let mtime = md.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            scan.files.push((path.to_path_buf(), mtime, md.len()));
        } else {
            scan.complete = false;
        }
    }
    scan
}

#[cfg(test)]
#[path = "cache_tests.rs"]
mod tests;
