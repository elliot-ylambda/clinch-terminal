//! Incremental file cache: parse a file only when its (mtime, size) changed.

use std::collections::HashMap;
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
}

impl<T> Default for ScanCache<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// Recursively list files under `root` whose name ends with `ext` (e.g. ".jsonl").
/// Missing/unreadable dir → empty vec (fail-soft).
pub fn scan_dir(root: &Path, ext: &str) -> Vec<(PathBuf, SystemTime, u64)> {
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if !path.to_string_lossy().ends_with(ext) {
            continue;
        }
        if let Ok(md) = entry.metadata() {
            let mtime = md.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            out.push((path.to_path_buf(), mtime, md.len()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::SystemTime;

    fn tmp() -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("cau_cache_{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn scan_dir_lists_only_matching_ext() {
        let d = tmp();
        fs::write(d.join("a.jsonl"), "x").unwrap();
        fs::write(d.join("b.txt"), "x").unwrap();
        fs::create_dir_all(d.join("sub")).unwrap();
        fs::write(d.join("sub/c.jsonl"), "x").unwrap();
        let mut found: Vec<_> = scan_dir(&d, ".jsonl")
            .into_iter()
            .map(|(p, _, _)| p)
            .collect();
        found.sort();
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|p| p.extension().unwrap() == "jsonl"));
    }

    #[test]
    fn scan_dir_missing_is_empty() {
        assert!(scan_dir(std::path::Path::new("/no/such/dir/xyz"), ".jsonl").is_empty());
    }

    #[test]
    fn cache_reparses_only_on_change() {
        let mut c: ScanCache<u32> = ScanCache::new();
        let p = std::path::Path::new("/fake/x.jsonl");
        let calls = std::cell::Cell::new(0u32);
        let m1 = SystemTime::UNIX_EPOCH;
        let v = *c.get_or_parse(p, m1, 10, |_| {
            calls.set(calls.get() + 1);
            42
        });
        assert_eq!(v, 42);
        // same mtime+size -> no re-parse
        let _ = c.get_or_parse(p, m1, 10, |_| {
            calls.set(calls.get() + 1);
            99
        });
        assert_eq!(calls.get(), 1);
        // changed size -> re-parse
        let v2 = *c.get_or_parse(p, m1, 11, |_| {
            calls.set(calls.get() + 1);
            7
        });
        assert_eq!(v2, 7);
        assert_eq!(calls.get(), 2);
    }
}
