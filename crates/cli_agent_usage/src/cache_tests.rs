use std::fs;
use std::time::SystemTime;

use super::*;

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
    // changed mtime alone (same size) -> re-parse
    let m2 = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(5);
    let v3 = *c.get_or_parse(p, m2, 11, |_| {
        calls.set(calls.get() + 1);
        13
    });
    assert_eq!(v3, 13);
    assert_eq!(calls.get(), 3);
}

#[test]
fn complete_scan_releases_deleted_sources_and_preserves_unchanged_values() {
    let dir = std::env::temp_dir().join(format!("cau_cache_prune_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let keep = dir.join("keep.jsonl");
    let deleted = dir.join("deleted.jsonl");
    fs::write(&keep, "keep").unwrap();
    fs::write(&deleted, "deleted").unwrap();

    let retained = std::rc::Rc::new(());
    let removed = std::rc::Rc::new(());
    let mut cache = ScanCache::new();
    for (path, mtime, size) in cache.scan_dir(&dir, ".jsonl") {
        cache.get_or_parse(&path, mtime, size, |path| {
            if path == keep {
                retained.clone()
            } else {
                removed.clone()
            }
        });
    }
    assert_eq!(std::rc::Rc::strong_count(&retained), 2);
    assert_eq!(std::rc::Rc::strong_count(&removed), 2);

    fs::remove_file(&deleted).unwrap();
    let files = cache.scan_dir(&dir, ".jsonl");
    assert_eq!(files.len(), 1);
    assert_eq!(std::rc::Rc::strong_count(&removed), 1);
    assert_eq!(std::rc::Rc::strong_count(&retained), 2);
    let (path, mtime, size) = &files[0];
    cache.get_or_parse(path, *mtime, *size, |_| {
        panic!("unchanged file was reparsed")
    });

    fs::remove_file(&keep).unwrap();
    assert!(cache.scan_dir(&dir, ".jsonl").is_empty());
    assert_eq!(std::rc::Rc::strong_count(&retained), 1);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn incomplete_scan_keeps_values_for_paths_it_could_not_visit() {
    let mut cache = ScanCache::new();
    let visible = Path::new("visible.jsonl");
    let inaccessible = Path::new("unreadable/retained.jsonl");
    cache.get_or_parse(visible, SystemTime::UNIX_EPOCH, 1, |_| 1);
    cache.get_or_parse(inaccessible, SystemTime::UNIX_EPOCH, 1, |_| 2);
    cache.retain_scanned_files(&DirectoryScan {
        files: vec![(visible.to_owned(), SystemTime::UNIX_EPOCH, 1)],
        complete: false,
    });
    assert_eq!(cache.entries.len(), 2);
    assert_eq!(
        *cache.get_or_parse(inaccessible, SystemTime::UNIX_EPOCH, 1, |_| {
            panic!("partial scan evicted an inaccessible source")
        }),
        2
    );
}

#[test]
fn failed_directory_walk_does_not_evict_cached_values() {
    let missing = std::env::temp_dir().join(format!("cau_cache_missing_{}", std::process::id()));
    let _ = fs::remove_dir_all(&missing);
    let mut cache = ScanCache::new();
    cache.get_or_parse(
        &missing.join("saved.jsonl"),
        SystemTime::UNIX_EPOCH,
        1,
        |_| 7,
    );
    assert!(cache.scan_dir(&missing, ".jsonl").is_empty());
    assert_eq!(cache.entries.len(), 1);
    assert!(!scan_dir_with_status(&missing, ".jsonl").complete);
}
