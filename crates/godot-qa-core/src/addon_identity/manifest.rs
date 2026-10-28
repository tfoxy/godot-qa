use crate::hash::{Sha256, to_lower_hex};
use std::fs;
use std::io::{self, Read};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

/// File order and length-prefixed fields make this independent of directory order,
/// native path separators, Git metadata, and ambiguous concatenations.
pub fn fingerprint(entries: &[Entry]) -> String {
    let mut hash = Sha256::new();
    hash.update(b"godot-qa.addon-identity.v1\0");
    hash.update(&(entries.len() as u64).to_be_bytes());
    for entry in entries {
        hash.update(&(entry.path.len() as u64).to_be_bytes());
        hash.update(entry.path.as_bytes());
        hash.update(&entry.bytes.to_be_bytes());
        hash.update(&(entry.sha256.len() as u64).to_be_bytes());
        hash.update(entry.sha256.as_bytes());
    }
    to_lower_hex(&hash.finish())
}

pub fn collect(root: &Path) -> io::Result<Vec<Entry>> {
    // Canonicalizing once permits a symlinked addon root without following arbitrary
    // directory symlinks below it (which could introduce cycles or external scripts).
    let root = fs::canonicalize(root)?;
    if !root.is_dir() {
        return Err(io::Error::other("addon root is not a directory"));
    }
    let mut entries = Vec::new();
    walk(&root, &root, &mut entries)?;
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(entries)
}

fn walk(root: &Path, directory: &Path, entries: &mut Vec<Entry>) -> io::Result<()> {
    for child in fs::read_dir(directory).map_err(|error| at_path(directory, error))? {
        let child = child?;
        let path = child.path();
        let kind = child.file_type()?;
        if kind.is_dir() {
            walk(root, &path, entries)?;
            continue;
        }
        let selected = path.extension().is_some_and(|extension| extension == "gd")
            || (directory == root && child.file_name() == "plugin.cfg");
        if kind.is_symlink() && (selected || path.is_dir()) {
            return Err(io::Error::other(format!(
                "cannot attest internal addon symlink {}",
                path.display()
            )));
        }
        if !selected {
            continue;
        }
        if !kind.is_file() {
            return Err(io::Error::other(format!(
                "addon source is not a regular file: {}",
                path.display()
            )));
        }
        let relative = path.strip_prefix(root).expect("walk stays below root");
        let mut parts = Vec::new();
        for component in relative.components() {
            parts.push(component.as_os_str().to_str().ok_or_else(|| {
                io::Error::other(format!(
                    "addon source path is not UTF-8: {}",
                    path.display()
                ))
            })?);
        }
        let mut file = fs::File::open(&path).map_err(|error| at_path(&path, error))?;
        let mut hash = Sha256::new();
        let mut bytes = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let count = file
                .read(&mut buffer)
                .map_err(|error| at_path(&path, error))?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
            bytes += count as u64;
        }
        entries.push(Entry {
            path: parts.join("/"),
            bytes,
            sha256: to_lower_hex(&hash.finish()),
        });
    }
    Ok(())
}

fn at_path(path: &Path, error: io::Error) -> io::Error {
    io::Error::new(error.kind(), format!("{}: {error}", path.display()))
}
