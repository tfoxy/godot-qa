//! Directory convenience links that do not require Windows symlink privileges.
use std::io;
use std::path::{Path, PathBuf};

pub fn read(path: &Path) -> io::Result<PathBuf> {
    #[cfg(windows)]
    {
        junction::get_target(path).or_else(|_| std::fs::read_link(path))
    }
    #[cfg(not(windows))]
    {
        std::fs::read_link(path)
    }
}

/// Reparse points include junctions, which must not be traversed by artifact pruning.
pub fn is_link(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

/// Replace only the link, never the directory it points to.
pub fn replace(target: &Path, link: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        // Windows cannot rename over a directory. Serialize the final remove/rename
        // across CLI processes; the lock is released by the OS even after a crash.
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let parent = link
            .parent()
            .ok_or_else(|| io::Error::other("directory link has no parent"))?;
        std::fs::create_dir_all(parent)?;
        use std::os::windows::fs::OpenOptionsExt;
        let lock_path = link.with_extension("lock");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let _lock = loop {
            match std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .share_mode(0)
                .open(&lock_path)
            {
                Ok(file) => break file,
                Err(error)
                    if error.raw_os_error() == Some(32) && std::time::Instant::now() < deadline =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(10))
                }
                Err(error) => return Err(error),
            }
        };
        let temp = parent.join(format!(
            ".latest-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let target = if target.is_absolute() {
            target.to_owned()
        } else {
            std::env::current_dir()?.join(target)
        };
        junction::create(&target, &temp)?;
        let result = (|| {
            if let Ok(metadata) = std::fs::symlink_metadata(link) {
                if is_link(&metadata) {
                    // RemoveDirectory removes a directory reparse point itself, even
                    // when its target is missing; it never traverses the target.
                    std::fs::remove_dir(link)?;
                } else if metadata.is_dir() {
                    // Legacy pointer directories contain only latest.json. Never remove
                    // an arbitrary artifact tree merely to install a convenience link.
                    let entries = std::fs::read_dir(link)?.collect::<Result<Vec<_>, _>>()?;
                    if entries
                        .iter()
                        .any(|entry| entry.file_name() != "latest.json")
                    {
                        return Err(io::Error::other(
                            "latest is a nonempty directory, not an owned convenience link",
                        ));
                    }
                    for entry in entries {
                        std::fs::remove_file(entry.path())?;
                    }
                    std::fs::remove_dir(link)?;
                } else {
                    std::fs::remove_file(link)?;
                }
            }
            std::fs::rename(&temp, link)
        })();
        if result.is_err() {
            let _ = std::fs::remove_dir(&temp);
        }
        result
    }
    #[cfg(not(windows))]
    {
        let _ = (target, link);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "use atomic Unix symlink replacement",
        ))
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn junction_rotation_preserves_targets_and_replaces_dangling_links() {
        let root = std::env::temp_dir().join(format!(
            "godot-qa-junction-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let a = root.join("first target");
        let b = root.join("second target");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("keep"), b"first").unwrap();
        std::fs::write(b.join("keep"), b"second").unwrap();
        let latest = root.join("latest");
        replace(&a, &latest).unwrap();
        assert_eq!(std::fs::read(latest.join("keep")).unwrap(), b"first");
        replace(&b, &latest).unwrap();
        assert_eq!(std::fs::read(latest.join("keep")).unwrap(), b"second");
        assert_eq!(std::fs::read(a.join("keep")).unwrap(), b"first");
        std::fs::remove_dir_all(&b).unwrap();
        replace(&a, &latest).unwrap();
        assert_eq!(std::fs::read(latest.join("keep")).unwrap(), b"first");
        std::fs::remove_dir(&latest).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
