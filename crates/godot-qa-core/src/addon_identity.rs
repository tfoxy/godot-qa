//! Content identity of the addon installed on disk, compared with the addon used
//! when this CLI was built. This does not attest an already-running Godot session.

mod manifest;
use manifest::{Entry, collect, fingerprint};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

include!(concat!(env!("OUT_DIR"), "/addon_identity.rs"));

#[derive(Clone, Debug, Serialize)]
pub struct ManifestSummary {
    pub sha256: String,
    pub files: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct AddonIdentityComparison {
    pub schema_version: u32,
    pub status: &'static str,
    pub scope: &'static str,
    pub path: String,
    pub expected: ManifestSummary,
    pub installed: Option<ManifestSummary>,
    pub missing: Vec<String>,
    pub extra: Vec<String>,
    pub changed: Vec<String>,
    pub errors: Vec<String>,
}

/// The expected identity is embedded; no source checkout or Git is consulted.
pub fn expected_manifest() -> ManifestSummary {
    ManifestSummary {
        sha256: EXPECTED_FINGERPRINT.to_owned(),
        files: EXPECTED_FILES.len(),
    }
}

pub fn compare_installed(root: &Path) -> AddonIdentityComparison {
    let mut comparison = AddonIdentityComparison {
        schema_version: 1,
        status: "unknown",
        scope: "installed_addon_content",
        path: root.to_string_lossy().into_owned(),
        expected: expected_manifest(),
        installed: None,
        missing: Vec::new(),
        extra: Vec::new(),
        changed: Vec::new(),
        errors: Vec::new(),
    };
    let entries = match collect(root) {
        Ok(entries) => entries,
        Err(error) => {
            comparison.errors.push(error.to_string());
            return comparison;
        }
    };
    comparison.installed = Some(ManifestSummary {
        sha256: fingerprint(&entries),
        files: entries.len(),
    });
    let expected: BTreeMap<_, _> = EXPECTED_FILES
        .iter()
        .map(|(path, bytes, hash)| (*path, (*bytes, *hash)))
        .collect();
    let actual: BTreeMap<_, _> = entries
        .iter()
        .map(|entry| (entry.path.as_str(), entry))
        .collect();
    for (path, (bytes, hash)) in &expected {
        match actual.get(path) {
            None => comparison.missing.push((*path).to_owned()),
            Some(Entry {
                bytes: actual_bytes,
                sha256,
                ..
            }) if actual_bytes != bytes || sha256 != hash => {
                comparison.changed.push((*path).to_owned())
            }
            _ => {}
        }
    }
    comparison.extra = actual
        .keys()
        .filter(|path| !expected.contains_key(**path))
        .map(|path| (*path).to_owned())
        .collect();
    comparison.status = if comparison.missing.is_empty()
        && comparison.extra.is_empty()
        && comparison.changed.is_empty()
    {
        "match"
    } else {
        "mismatch"
    };
    comparison
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn directory() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "godot-qa-identity-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn sorted_manifest_selects_only_runtime_scripts_and_root_plugin() {
        let root = directory();
        fs::create_dir_all(root.join("sub")).unwrap();
        for (path, bytes) in [
            ("z.gd", "z"),
            ("plugin.cfg", "plugin"),
            ("sub/a.gd", "a"),
            ("sub/plugin.cfg", "ignored"),
            ("README.md", "ignored"),
            ("z.gd.uid", "ignored"),
        ] {
            fs::write(root.join(path), bytes).unwrap();
        }
        let entries = collect(&root).unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.path.as_str())
                .collect::<Vec<_>>(),
            ["plugin.cfg", "sub/a.gd", "z.gd"]
        );
        let identity = fingerprint(&entries);
        fs::write(root.join("README.md"), "changed docs").unwrap();
        assert_eq!(fingerprint(&collect(&root).unwrap()), identity);
        fs::write(root.join("sub/a.gd"), "changed source").unwrap();
        assert_ne!(fingerprint(&collect(&root).unwrap()), identity);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn comparison_reports_missing_extra_and_changed_at_equal_version() {
        let root = directory();
        fs::write(root.join("plugin.cfg"), "[plugin]\nversion=\"0.1.0\"\n").unwrap();
        fs::write(root.join("extra.gd"), "extends Node\n").unwrap();
        let comparison = compare_installed(&root);
        assert_eq!(comparison.status, "mismatch");
        assert_eq!(comparison.extra, ["extra.gd"]);
        assert!(comparison.changed.contains(&"plugin.cfg".to_string()));
        assert!(comparison.missing.iter().any(|path| path.ends_with(".gd")));
        assert!(comparison.errors.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_or_non_directory_root_is_unknown() {
        let root = directory();
        fs::write(root.join("file"), "not a directory").unwrap();
        for path in [root.join("missing"), root.join("file")] {
            let comparison = compare_installed(&path);
            assert_eq!(comparison.status, "unknown");
            assert!(comparison.installed.is_none());
            assert!(!comparison.errors.is_empty());
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invocation_clones_share_one_comparison_but_new_invocations_refresh() {
        let root = directory();
        fs::write(root.join("project.godot"), "[application]\n").unwrap();
        let addon = root.join("addons/godot_qa");
        fs::create_dir_all(&addon).unwrap();
        let config = crate::config::load_project_config(Some(&root), None, None, false, None);
        let children = [config.clone(), config.clone()];
        let first = config.installed_addon_identity();
        fs::write(addon.join("new.gd"), "extends Node\n").unwrap();
        std::thread::scope(|scope| {
            for child in &children {
                scope.spawn(|| {
                    assert!(std::ptr::eq(first, child.installed_addon_identity()));
                    assert!(
                        !child
                            .installed_addon_identity()
                            .extra
                            .contains(&"new.gd".to_owned())
                    );
                });
            }
        });
        let next = crate::config::load_project_config(Some(&root), None, None, false, None);
        assert!(
            next.installed_addon_identity()
                .extra
                .contains(&"new.gd".to_owned())
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlink_root_is_allowed_but_unreadable_internal_source_is_unknown() {
        use std::os::unix::fs::symlink;
        let root = directory();
        fs::create_dir(root.join("addon")).unwrap();
        fs::write(root.join("addon/a.gd"), "a").unwrap();
        symlink(root.join("addon"), root.join("linked")).unwrap();
        assert_eq!(
            collect(&root.join("addon")).unwrap(),
            collect(&root.join("linked")).unwrap()
        );
        symlink(root.join("missing.gd"), root.join("addon/unreadable.gd")).unwrap();
        assert_eq!(compare_installed(&root.join("linked")).status, "unknown");
        fs::remove_dir_all(root).unwrap();
    }
}
