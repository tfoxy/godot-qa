use super::*;
use std::collections::BTreeMap;
use std::io::Write;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Journal {
    pub format: String,
    pub id: String,
    pub project: PathBuf,
    pub manifest: Manifest,
    pub scenario: Value,
    pub execution_config: Value,
    pub candidate_hashes: Vec<String>,
    pub source_paths: Vec<PathBuf>,
    pub state: String,
    pub phase: String,
    pub intent: Option<String>,
    pub original_hash: String,
    pub output_hash: Option<String>,
    pub scene_uid: Option<String>,
    pub owned: BTreeMap<String, String>,
    pub media_captures: BTreeMap<String, String>,
    pub frozen: BTreeMap<String, String>,
    pub evidence_root: PathBuf,
    pub evidence_hashes: BTreeMap<String, String>,
    pub baseline: Option<Value>,
    pub candidate: Option<Value>,
    pub import_probe: Option<Value>,
    pub failures: Vec<FailureDetail>,
    pub warnings: Vec<String>,
}
impl Journal {
    pub fn dir(&self) -> PathBuf {
        self.project.join(".qa/asset-trials").join(&self.id)
    }
    pub fn terminal(&self) -> bool {
        matches!(self.state.as_str(), "kept" | "discarded")
    }
    pub fn save_journal(&self) -> Result<(), FailureDetail> {
        let mut payload = json!(self);
        let digest = godot_qa_core::hash::sha256_hex(&serde_json::to_vec(&payload).unwrap());
        payload["integrity_sha256"] = json!(digest);
        atomic_json(&self.dir().join("journal.json"), &payload)?;
        let review_path = self.dir().join("review.json");
        let previous = if review_path.exists() {
            read_json(&review_path).unwrap_or(Value::Null)
        } else {
            Value::Null
        };
        atomic_json(
            &review_path,
            &json!({"format":"godot-qa.asset-trial-review.v1","trial_id":self.id,"state":self.state,"phase":self.phase,"technical_status":if self.phase=="evaluated" && self.failures.is_empty(){"pass"}else if self.failures.is_empty(){"incomplete"}else{"fail"},"review_status":"unreviewed","before":self.baseline,"after":self.candidate,"import_probe":self.import_probe,"bindings":self.manifest.bindings,"comparisons":previous["comparisons"],"failures":self.failures,"warnings":self.warnings,"journal":self.dir().join("journal.json"),"backup":self.dir().join("original.tscn")}),
        )?;
        Ok(())
    }
    pub fn save(&self) -> Result<(), FailureDetail> {
        self.save_journal()?;
        if !self.terminal() {
            atomic_json(
                &self.project.join(".qa/asset-trials/active.json"),
                &json!({"trial_id":self.id,"state":self.state,"coordinator_pid":std::process::id()}),
            )?;
        }
        Ok(())
    }
    pub fn step(&mut self, phase: &str) -> Result<(), FailureDetail> {
        self.phase = phase.to_owned();
        self.state = phase.to_owned();
        self.intent = None;
        self.save()
    }
    pub fn intent(&mut self, operation: &str) -> Result<(), FailureDetail> {
        self.intent = Some(operation.to_owned());
        self.save()
    }
    pub fn receipt(&mut self, operation: &str) -> Result<(), FailureDetail> {
        self.intent = Some(operation.to_owned());
        self.save()
    }
    pub fn clear_active(&self) -> Result<(), FailureDetail> {
        let path = self.project.join(".qa/asset-trials/active.json");
        if path.exists() {
            let marker: Value = read_json(&path)?;
            if marker["trial_id"] != self.id {
                return Err(fail(
                    "asset_trial_reservation_conflict",
                    &self.id,
                    "active reservation belongs to another trial",
                ));
            }
            fs::remove_file(&path).map_err(|e| io_failure(&path, e))?;
            sync_dir(path.parent().unwrap())?;
        }
        Ok(())
    }
}
pub(super) fn read_json(path: &Path) -> Result<Value, FailureDetail> {
    serde_json::from_slice(&manifest::bounded_read(path, 64 * 1024 * 1024)?).map_err(|e| {
        fail(
            "asset_trial_journal_invalid",
            &path.display().to_string(),
            &e.to_string(),
        )
    })
}
pub(super) fn atomic_json(path: &Path, value: &Value) -> Result<(), FailureDetail> {
    let tmp = path.with_extension(format!("tmp-{}", random_id()?));
    write_new(
        &tmp,
        &serde_json::to_vec_pretty(value).map_err(|e| {
            fail(
                "asset_trial_journal_invalid",
                &path.display().to_string(),
                &e.to_string(),
            )
        })?,
    )?;
    fs::rename(&tmp, path).map_err(|e| io_failure(path, e))?;
    sync_dir(path.parent().unwrap())
}
pub(super) fn write_new(path: &Path, bytes: &[u8]) -> Result<(), FailureDetail> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| io_failure(path, e))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| io_failure(path, e))?;
    sync_dir(path.parent().unwrap())
}
pub(super) fn sync_dir(path: &Path) -> Result<(), FailureDetail> {
    #[cfg(unix)]
    fs::File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(|e| io_failure(path, e))?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
pub(super) fn file_hash(path: &Path) -> Result<String, FailureDetail> {
    Ok(godot_qa_core::hash::sha256_hex(&manifest::bounded_read(
        path,
        256 * 1024 * 1024,
    )?))
}
pub(super) fn check_hash(path: &Path, expected: &str) -> Result<(), FailureDetail> {
    let actual = file_hash(path)?;
    if actual != expected {
        let mut failure = fail(
            "asset_trial_conflict",
            &path.display().to_string(),
            "file changed; preserve all versions and discard only after resolving the conflict",
        );
        failure.expected = Some(json!(expected));
        failure.actual = Some(json!(actual));
        return Err(failure);
    }
    Ok(())
}
pub(super) fn load(config: &ProjectConfig, id: &str) -> Result<Journal, FailureDetail> {
    if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(fail(
            "asset_trial_id_invalid",
            id,
            "expected a 32-character trial ID",
        ));
    }
    let path = config
        .project_root
        .join(".qa/asset-trials")
        .join(id)
        .join("journal.json");
    manifest::no_links(&config.project_root, &path)?;
    let mut payload = read_json(&path)?;
    let integrity = payload
        .as_object_mut()
        .and_then(|object| object.remove("integrity_sha256"))
        .and_then(|value| value.as_str().map(str::to_owned));
    let actual = godot_qa_core::hash::sha256_hex(&serde_json::to_vec(&payload).unwrap());
    if integrity.as_deref() != Some(actual.as_str()) {
        return Err(fail(
            "asset_trial_journal_invalid",
            &path.display().to_string(),
            "journal integrity checksum mismatch; preserve all files and repair from the original journal before recovery",
        ));
    }
    let journal: Journal = serde_json::from_value(payload).map_err(|e| {
        fail(
            "asset_trial_journal_invalid",
            &path.display().to_string(),
            &e.to_string(),
        )
    })?;
    if journal.format != "godot-qa.asset-trial-journal.v1"
        || journal.id != id
        || journal.project != config.project_root
    {
        return Err(fail(
            "asset_trial_journal_invalid",
            id,
            "journal identity does not match selected canonical project",
        ));
    }
    manifest::project_path(&journal.project, &journal.manifest.scene)?;
    for resource in journal.owned.keys() {
        manifest::project_path(&journal.project, resource)?;
    }
    for (resource, name) in &journal.media_captures {
        if !journal.owned.contains_key(resource)
            || !name.starts_with("custody-")
            || !name.ends_with(".bin")
            || name.len() != 43
            || !name.as_bytes()[7..39].iter().all(|b| b.is_ascii_hexdigit())
        {
            return Err(fail(
                "asset_trial_journal_invalid",
                resource,
                "invalid media custody entry",
            ));
        }
        manifest::no_links(&journal.project, &journal.dir().join(name))?;
    }
    // Only immutable media declared by the manifest, and their adjacent sidecars, may be removed.
    for resource in journal.owned.keys() {
        if !journal.manifest.bindings.iter().any(|b| {
            resource == &b.destination
                || resource == &format!("{}.import", b.destination)
                || resource == &format!("{}.uid", b.destination)
        }) {
            return Err(fail(
                "asset_trial_journal_invalid",
                resource,
                "ownership entry is not a manifest destination or sidecar",
            ));
        }
    }
    if journal.candidate_hashes.len() != journal.manifest.bindings.len()
        || journal.source_paths.len() != journal.candidate_hashes.len()
    {
        return Err(fail(
            "asset_trial_journal_invalid",
            id,
            "candidate hash inventory differs from bindings",
        ));
    }
    manifest::no_links(&journal.project, &journal.dir().join("original.tscn"))?;
    for (index, hash) in journal.candidate_hashes.iter().enumerate() {
        manifest::no_links(
            &journal.project,
            &journal.dir().join(format!("candidate-{index}.bin")),
        )?;
        check_hash(&journal.dir().join(format!("candidate-{index}.bin")), hash)?;
    }
    for relative in journal.frozen.keys() {
        manifest::no_links(&journal.project, &journal.project.join(relative))?;
    }
    for relative in journal.evidence_hashes.keys() {
        manifest::no_links(
            &journal.evidence_root,
            &journal.evidence_root.join(relative),
        )?;
    }
    check_hash(&journal.dir().join("original.tscn"), &journal.original_hash)?;
    Ok(journal)
}

/// Move/capture first, rehash the bytes actually captured, then publish with a
/// hard link. Neither forward nor rollback publication can overwrite a race winner.
/// The hook is the deterministic filesystem fault boundary used by crash tests.
pub(super) fn replace_no_clobber(
    journal: &mut Journal,
    target: &Path,
    replacement: &Path,
    expected: &str,
    prefix: &str,
    hook: &mut dyn FnMut(&str) -> Result<(), FailureDetail>,
) -> Result<(), FailureDetail> {
    let capture = journal.dir().join(format!("{prefix}-captured.tscn"));
    if capture.exists() {
        return Err(fail(
            "asset_trial_recovery_required",
            &capture.display().to_string(),
            "captured bytes already exist; recover the recorded operation before retrying",
        ));
    }
    journal.intent(&format!("{prefix}_capture_intent"))?;
    hook("capture_intent")?;
    fs::rename(target, &capture).map_err(|e| io_failure(target, e))?;
    sync_dir(target.parent().unwrap())?;
    sync_dir(&journal.dir())?;
    hook("capture_moved")?;
    journal.receipt(&format!("{prefix}_captured"))?;
    hook("captured")?;
    if let Err(failure) = check_hash(&capture, expected) {
        // A concurrent in-place edit is preserved verbatim, including if another
        // writer already recreated the public path. Never copy over that writer.
        if fs::symlink_metadata(target).is_err() {
            fs::hard_link(&capture, target).map_err(|e| io_failure(target, e))?;
            sync_dir(target.parent().unwrap())?;
        }
        return Err(failure);
    }
    journal.intent(&format!("{prefix}_publish_intent"))?;
    hook("publish_intent")?;
    fs::hard_link(replacement, target).map_err(|e| io_failure(target, e))?;
    sync_dir(target.parent().unwrap())?;
    hook("published_link")?;
    journal.receipt(&format!("{prefix}_published"))?;
    hook("published")?;
    Ok(())
}

pub(super) fn snapshot_tree(
    root: &Path,
    exclude: &[PathBuf],
) -> Result<BTreeMap<String, String>, FailureDetail> {
    fn walk(
        root: &Path,
        dir: &Path,
        exclude: &[PathBuf],
        out: &mut BTreeMap<String, String>,
    ) -> Result<(), FailureDetail> {
        for entry in fs::read_dir(dir).map_err(|e| io_failure(dir, e))? {
            let entry = entry.map_err(|e| io_failure(dir, e))?;
            let path = entry.path();
            if exclude.iter().any(|p| path.starts_with(p)) {
                continue;
            }
            let meta = fs::symlink_metadata(&path).map_err(|e| io_failure(&path, e))?;
            if godot_qa_process::directory_link::is_link(&meta) {
                return Err(fail(
                    "asset_trial_path_invalid",
                    &path.display().to_string(),
                    "symlink dependencies cannot be frozen",
                ));
            }
            if meta.is_dir() {
                if matches!(entry.file_name().to_str(), Some(".git" | ".godot" | ".qa")) {
                    continue;
                }
                walk(root, &path, exclude, out)?;
            } else if meta.is_file() {
                out.insert(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                    file_hash(&path)?,
                );
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    walk(root, root, exclude, &mut out)?;
    Ok(out)
}
pub(super) fn verify_frozen(journal: &Journal) -> Result<(), FailureDetail> {
    for (path, hash) in journal.source_paths.iter().zip(&journal.candidate_hashes) {
        check_hash(path, hash)?;
    }
    let excluded = journal.execution_config["artifacts_dir"]
        .as_str()
        .map(PathBuf::from)
        .into_iter()
        .collect::<Vec<_>>();
    let current = snapshot_tree(&journal.project, &excluded)?;
    for path in current.keys() {
        if !journal.frozen.contains_key(path)
            && path != journal.manifest.scene.trim_start_matches("res://")
            && !journal.owned.contains_key(&format!("res://{path}"))
        {
            return Err(fail(
                "asset_trial_noncomparable",
                path,
                "new source/dependency appeared after baseline was frozen",
            ));
        }
    }
    for (relative, hash) in &journal.frozen {
        let path = journal.project.join(relative);
        manifest::no_links(&journal.project, &path)?;
        check_hash(&path, hash)?;
    }
    Ok(())
}
