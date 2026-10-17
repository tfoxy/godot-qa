//! Process-wide cache for `project.godot` reads.
//!
//! `project.godot` is read repeatedly on hot paths that are not obviously
//! related: frame-pacing resolution, the Wayland preference probe, session
//! main-scene resolution, and scenario main-scene resolution all re-read the
//! same file, and a warm bundle multiplies that by every scenario and every
//! lane in the process. The file is small, but the reads are pure syscall
//! overhead once the contents are known.
//!
//! The cache is keyed on the file's `(mtime, len)` stamp taken from a single
//! `fs::metadata` call, so an out-of-band rewrite — including
//! `diagnostics editor`, which mutates `project.godot` in-process and then
//! restores it — invalidates the entry instead of serving stale content. When
//! the stamp cannot be taken at all the read falls through uncached, so a
//! missing stamp degrades to today's behaviour rather than to a stale answer.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

type CacheEntry = (SystemTime, u64, Arc<String>);

fn cache() -> &'static Mutex<HashMap<PathBuf, CacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, CacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Read `project.godot` (or any small project file) through the process-wide
/// cache, returning `None` when the file cannot be read.
///
/// A cached entry is reused only when the file's current `(mtime, len)` stamp
/// still matches the stamp recorded with it, so callers observe rewrites. The
/// returned `Arc` is shared with every other caller holding the same stamp;
/// callers only parse the contents, so sharing is safe and avoids re-reading.
pub fn read_project_godot_cached(path: &Path) -> Option<Arc<String>> {
    // One metadata call supplies both halves of the stamp. Any failure here
    // (missing file, permission error, a platform without mtime) means there is
    // no key to cache under, so read straight through.
    let Some(stamp) = fs::metadata(path)
        .ok()
        .and_then(|metadata| Some((metadata.modified().ok()?, metadata.len())))
    else {
        return fs::read_to_string(path).ok().map(Arc::new);
    };

    if let Some((cached_mtime, cached_len, contents)) = lock_cache().get(path)
        && (*cached_mtime, *cached_len) == stamp
    {
        return Some(Arc::clone(contents));
    }

    let contents = Arc::new(fs::read_to_string(path).ok()?);
    lock_cache().insert(
        path.to_path_buf(),
        (stamp.0, stamp.1, Arc::clone(&contents)),
    );
    Some(contents)
}

/// The cache holds no invariant that a panic could break, so a poisoned lock is
/// recovered instead of turning every later read into a panic.
fn lock_cache() -> std::sync::MutexGuard<'static, HashMap<PathBuf, CacheEntry>> {
    cache().lock().unwrap_or_else(|error| error.into_inner())
}

#[cfg(test)]
mod tests {
    use super::read_project_godot_cached;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_project(label: &str) -> PathBuf {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "godot-qa-project-file-{label}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp project dir");
        dir
    }

    #[test]
    fn repeated_reads_share_one_cached_allocation() {
        let dir = temp_project("cache-hit");
        let path = dir.join("project.godot");
        fs::write(&path, "config_version=5\n").expect("write project");

        let first = read_project_godot_cached(&path).expect("first read");
        let second = read_project_godot_cached(&path).expect("second read");

        assert_eq!(first.as_str(), "config_version=5\n");
        assert!(
            std::sync::Arc::ptr_eq(&first, &second),
            "an unchanged project.godot must be served from the cache"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_rewrite_of_a_different_length_is_observed() {
        let dir = temp_project("length-change");
        let path = dir.join("project.godot");
        fs::write(&path, "config_version=5\n").expect("write project");
        let first = read_project_godot_cached(&path).expect("first read");

        // `diagnostics editor` rewrites project.godot in-process; the (mtime, len)
        // stamp must cover that, so the next read must return the new content.
        fs::write(&path, "config_version=5\n\n[application]\n").expect("rewrite project");
        let second = read_project_godot_cached(&path).expect("second read");

        assert_eq!(first.as_str(), "config_version=5\n");
        assert_eq!(second.as_str(), "config_version=5\n\n[application]\n");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_same_length_rewrite_is_observed_through_the_mtime() {
        let dir = temp_project("mtime-change");
        let path = dir.join("project.godot");
        fs::write(&path, "run/main_scene=\"res://a.tscn\"\n").expect("write project");
        let first = read_project_godot_cached(&path).expect("first read");

        // Same byte length: only the mtime half of the stamp can catch this, so
        // give the filesystem clock room to tick before rewriting.
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&path, "run/main_scene=\"res://b.tscn\"\n").expect("rewrite project");
        let second = read_project_godot_cached(&path).expect("second read");

        assert_eq!(first.as_str(), "run/main_scene=\"res://a.tscn\"\n");
        assert_eq!(second.as_str(), "run/main_scene=\"res://b.tscn\"\n");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_project_file_reads_as_none() {
        let dir = temp_project("missing");
        let path = dir.join("project.godot");

        assert!(read_project_godot_cached(&path).is_none());

        // A file that appears later is still read, i.e. the miss is not cached.
        fs::write(&path, "config_version=5\n").expect("write project");
        assert_eq!(
            read_project_godot_cached(&path)
                .expect("read after creation")
                .as_str(),
            "config_version=5\n"
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
