//! Cooperative project leases for source-changing asset trials.
//!
//! A reservation survives the CLI, while OS leases cover launches and writers.
//! Coordinator authority is thread-local and never travels over the bridge.

use godot_qa_core::results::FailureDetail;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::rc::Rc;

thread_local! {
    static COORDINATORS: RefCell<Vec<PathBuf>> = const { RefCell::new(Vec::new()) };
    static LAUNCHES: RefCell<Vec<PathBuf>> = const { RefCell::new(Vec::new()) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchClass {
    RuntimePreview,
    ImportOrEditor,
    SourceWriter,
    CoordinatorChild,
}

fn failure(root: &Path, kind: &str, message: impl Into<String>, actual: Value) -> FailureDetail {
    FailureDetail {
        kind: kind.into(),
        command: Some("project access".into()),
        path: Some(
            root.join(".qa/asset-trials/active.json")
                .display()
                .to_string(),
        ),
        message: Some(message.into()),
        expected: Some(json!(
            "compatible project lease; stop previews or keep/discard the active trial"
        )),
        actual: Some(actual),
        ..FailureDetail::default()
    }
}

fn canonical(root: &Path) -> Result<PathBuf, FailureDetail> {
    fs::canonicalize(root)
        .map_err(|e| failure(root, "project_access_failed", e.to_string(), json!(root)))
}

fn lock_file(root: &Path) -> Result<File, FailureDetail> {
    // Never follow a user-replaced control directory or lock-file link.
    let mut path = root.to_path_buf();
    for part in [".qa", "asset-trials"] {
        path.push(part);
        if let Ok(meta) = fs::symlink_metadata(&path)
            && (crate::directory_link::is_link(&meta) || !meta.is_dir())
        {
            return Err(failure(
                root,
                "project_access_path_invalid",
                "project control paths must be ordinary directories",
                json!(path),
            ));
        }
        fs::create_dir_all(&path)
            .map_err(|e| failure(root, "project_access_failed", e.to_string(), json!(path)))?;
    }
    path.push("project.lock");
    if let Ok(meta) = fs::symlink_metadata(&path)
        && (crate::directory_link::is_link(&meta) || !meta.is_file())
    {
        return Err(failure(
            root,
            "project_access_path_invalid",
            "project lock must be an ordinary file",
            json!(path),
        ));
    }
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| failure(root, "project_access_failed", e.to_string(), json!(path)))
}

pub struct ProjectLease {
    _file: File,
}

impl Drop for ProjectLease {
    fn drop(&mut self) {
        // Release explicitly: another thread may have forked a child which
        // briefly retains this descriptor until exec closes it.
        let _ = self._file.unlock();
    }
}

impl ProjectLease {
    /// Reserve a project for a trial transition. Existing detached runtimes are
    /// checked while holding the lock that every detached launch also holds.
    pub fn exclusive(project: &Path) -> Result<Self, FailureDetail> {
        let root = canonical(project)?;
        let file = lock_file(&root)?;
        file.try_lock().map_err(|e| {
            failure(
                &root,
                "asset_trial_project_busy",
                e.to_string(),
                json!("active project operation"),
            )
        })?;
        let mut live = live_processes(&root)?;
        live.extend(external_project_processes(&root));
        if !live.is_empty() {
            return Err(failure(
                &root,
                "asset_trial_project_busy",
                "stop the listed project processes before changing source; no process was killed",
                json!(live),
            ));
        }
        Ok(Self { _file: file })
    }
}

/// Keep this scope on the same thread as the coordinator's exclusive lease.
pub struct CoordinatorScope {
    root: PathBuf,
    _not_send: PhantomData<Rc<()>>,
}

impl CoordinatorScope {
    pub fn enter(project: &Path) -> Self {
        let root = fs::canonicalize(project).unwrap_or_else(|_| project.to_path_buf());
        COORDINATORS.with(|paths| paths.borrow_mut().push(root.clone()));
        Self {
            root,
            _not_send: PhantomData,
        }
    }
}

impl Drop for CoordinatorScope {
    fn drop(&mut self) {
        COORDINATORS.with(|paths| {
            let mut paths = paths.borrow_mut();
            if let Some(index) = paths.iter().rposition(|p| p == &self.root) {
                paths.remove(index);
            }
        });
    }
}

pub struct LaunchLease {
    root: PathBuf,
    _file: Option<File>,
    _not_send: PhantomData<Rc<()>>,
}

impl LaunchLease {
    pub fn acquire(project: &Path, class: LaunchClass) -> Result<Self, FailureDetail> {
        let root = canonical(project)?;
        let coordinator = COORDINATORS.with(|paths| paths.borrow().contains(&root));
        let file = if coordinator {
            None
        } else {
            let file = lock_file(&root)?;
            file.try_lock_shared().map_err(|e| {
                failure(
                    &root,
                    "asset_trial_project_busy",
                    e.to_string(),
                    json!("exclusive project operation"),
                )
            })?;
            let marker = root.join(".qa/asset-trials/active.json");
            match fs::read(&marker) {
                Ok(bytes) => {
                    let active: Value = serde_json::from_slice(&bytes).map_err(|e| {
                        failure(
                            &root,
                            "asset_trial_journal_invalid",
                            e.to_string(),
                            json!(marker),
                        )
                    })?;
                    if class != LaunchClass::RuntimePreview
                        || active.get("state").and_then(Value::as_str) != Some("evaluated")
                    {
                        return Err(failure(
                            &root,
                            "asset_trial_active",
                            "this operation is incompatible with the active asset trial",
                            active,
                        ));
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(failure(
                        &root,
                        "project_access_failed",
                        e.to_string(),
                        json!(marker),
                    ));
                }
            }
            if class == LaunchClass::CoordinatorChild {
                return Err(failure(
                    &root,
                    "asset_trial_coordinator_required",
                    "trial child launch requires coordinator authority",
                    json!("no coordinator"),
                ));
            }
            Some(file)
        };
        LAUNCHES.with(|paths| paths.borrow_mut().push(root.clone()));
        Ok(Self {
            root,
            _file: file,
            _not_send: PhantomData,
        })
    }
}

impl Drop for LaunchLease {
    fn drop(&mut self) {
        if let Some(file) = &self._file {
            let _ = file.unlock();
        }
        LAUNCHES.with(|paths| {
            let mut paths = paths.borrow_mut();
            if let Some(index) = paths.iter().rposition(|p| p == &self.root) {
                paths.remove(index);
            }
        });
    }
}

/// Track the child before releasing the launch lease. Receipts contain no token
/// and remain after a coordinator crash, allowing recovery to refuse live work.
pub fn register_child(pid: i64) -> std::io::Result<Option<ChildRegistration>> {
    let Some(root) = LAUNCHES.with(|paths| paths.borrow().last().cloned()) else {
        return Ok(None);
    };
    let dir = root.join(".qa/asset-trials/processes");
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{pid}.json"));
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)?;
    serde_json::to_writer(
        &mut file,
        &json!({"pid":pid,"process_identity":tracked_identity(pid)}),
    )?;
    file.flush()?;
    file.sync_all()?;
    Ok(Some(ChildRegistration { path, pid }))
}

pub struct ChildRegistration {
    path: PathBuf,
    pid: i64,
}

impl Drop for ChildRegistration {
    fn drop(&mut self) {
        if !tracked_alive(self.pid) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn tracked_identity(pid: i64) -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let fields = stat.get(stat.rfind(')')? + 2..)?;
        fields.split_whitespace().nth(19)?.parse().ok()
    }
    #[cfg(not(target_os = "linux"))]
    {
        crate::process_identity(pid)
    }
}

fn tracked_alive(pid: i64) -> bool {
    #[cfg(target_os = "linux")]
    if let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat"))
        && let Some(end) = stat.rfind(')')
        && stat
            .get(end + 2..)
            .is_some_and(|s| s.starts_with('Z') || s.starts_with('X'))
    {
        return false;
    }
    crate::pid_is_running(pid)
}

fn live_processes(root: &Path) -> Result<Vec<Value>, FailureDetail> {
    let mut paths = vec![
        root.join(".qa/session.json"),
        root.join(".qa/debug_host.json"),
    ];
    let dir = root.join(".qa/asset-trials/processes");
    if dir.exists() {
        for entry in fs::read_dir(&dir)
            .map_err(|e| failure(root, "project_access_failed", e.to_string(), json!(dir)))?
        {
            paths.push(
                entry
                    .map_err(|e| failure(root, "project_access_failed", e.to_string(), json!(dir)))?
                    .path(),
            );
        }
    }
    let mut live = Vec::new();
    for path in paths {
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                return Err(failure(
                    root,
                    "project_access_failed",
                    e.to_string(),
                    json!(path),
                ));
            }
        };
        let record: Value = serde_json::from_slice(&bytes).map_err(|e| {
            failure(
                root,
                "asset_trial_process_record_invalid",
                e.to_string(),
                json!(path),
            )
        })?;
        if let Some(pid) = record.get("pid").and_then(Value::as_i64)
            && tracked_alive(pid)
            && record
                .get("process_identity")
                .and_then(Value::as_u64)
                .is_none_or(|expected| {
                    tracked_identity(pid).is_none_or(|actual| actual == expected)
                })
        {
            live.push(json!({"pid":pid,"record":path}));
        }
    }
    Ok(live)
}

/// Best effort only: external editors do not participate in our leases. On
/// Linux inspect accessible Godot command lines; never signal these processes.
fn external_project_processes(root: &Path) -> Vec<Value> {
    #[cfg(target_os = "linux")]
    {
        let mut found = Vec::new();
        let Ok(entries) = fs::read_dir("/proc") else {
            return found;
        };
        for entry in entries.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|s| s.parse::<i64>().ok())
            else {
                continue;
            };
            if !tracked_alive(pid) {
                continue;
            }
            let Ok(executable) = fs::read_link(entry.path().join("exe")) else {
                continue;
            };
            if !executable.file_name().is_some_and(|s| {
                let name = s.to_string_lossy().to_ascii_lowercase();
                name.starts_with("godot")
                    && !name.starts_with("godot-qa")
                    && !name.starts_with("godot_qa")
            }) {
                continue;
            }
            let Ok(bytes) = fs::read(entry.path().join("cmdline")) else {
                continue;
            };
            let args: Vec<_> = bytes
                .split(|b| *b == 0)
                .map(String::from_utf8_lossy)
                .collect();
            let cwd = fs::read_link(entry.path().join("cwd")).ok();
            let selected = args
                .windows(2)
                .find(|a| a[0] == "--path")
                .map(|a| PathBuf::from(a[1].as_ref()));
            let selected = selected
                .map(|p| {
                    if p.is_absolute() {
                        p
                    } else {
                        cwd.clone().unwrap_or_default().join(p)
                    }
                })
                .or(cwd);
            if selected.and_then(|p| fs::canonicalize(p).ok()).as_deref() == Some(root) {
                found.push(json!({"pid":pid,"external":true,"executable":executable}));
            }
        }
        found
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = root;
        Vec::new()
    }
}

pub fn class_with_engine_args(default: LaunchClass, args: &[String]) -> LaunchClass {
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "--editor" | "-e" | "--import"))
    {
        LaunchClass::ImportOrEditor
    } else {
        default
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    fn project() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "gqa-access-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }
    #[test]
    fn shared_previews_exclude_trial_transition_and_other_projects_are_independent() {
        let a = project();
        let b = project();
        let preview = LaunchLease::acquire(&a, LaunchClass::RuntimePreview).unwrap();
        assert!(ProjectLease::exclusive(&a).is_err());
        let independent = ProjectLease::exclusive(&b).unwrap();
        drop(preview);
        drop(independent);
        assert!(ProjectLease::exclusive(&a).is_ok());
        let _ = fs::remove_dir_all(a);
        let _ = fs::remove_dir_all(b);
    }
    #[test]
    fn reservation_state_controls_launches_and_coordinator_is_thread_local() {
        let root = project();
        let lease = ProjectLease::exclusive(&root).unwrap();
        fs::write(
            root.join(".qa/asset-trials/active.json"),
            br#"{"trial_id":"t","state":"staged"}"#,
        )
        .unwrap();
        let scope = CoordinatorScope::enter(&root);
        assert!(LaunchLease::acquire(&root, LaunchClass::ImportOrEditor).is_ok());
        let other = root.clone();
        assert!(
            std::thread::spawn(
                move || LaunchLease::acquire(&other, LaunchClass::RuntimePreview).is_err()
            )
            .join()
            .unwrap()
        );
        drop(scope);
        drop(lease);
        assert!(LaunchLease::acquire(&root, LaunchClass::RuntimePreview).is_err());
        fs::write(
            root.join(".qa/asset-trials/active.json"),
            br#"{"trial_id":"t","state":"evaluated"}"#,
        )
        .unwrap();
        assert!(LaunchLease::acquire(&root, LaunchClass::RuntimePreview).is_ok());
        assert!(LaunchLease::acquire(&root, LaunchClass::ImportOrEditor).is_err());
        assert!(LaunchLease::acquire(&root, LaunchClass::SourceWriter).is_err());
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn live_detached_receipt_blocks_recovery_and_corrupt_marker_blocks_launch() {
        let root = project();
        let launch = LaunchLease::acquire(&root, LaunchClass::RuntimePreview).unwrap();
        let receipt = register_child(i64::from(std::process::id())).unwrap();
        drop(launch);
        assert_eq!(
            ProjectLease::exclusive(&root).err().unwrap().kind,
            "asset_trial_project_busy"
        );
        drop(receipt);
        fs::write(root.join(".qa/asset-trials/active.json"), "bad").unwrap();
        assert_eq!(
            LaunchLease::acquire(&root, LaunchClass::RuntimePreview)
                .err()
                .unwrap()
                .kind,
            "asset_trial_journal_invalid"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn subprocess_preview_helper() {
        let Some(root) = std::env::var_os("GODOT_QA_LEASE_TEST_ROOT").map(PathBuf::from) else {
            return;
        };
        let _lease = LaunchLease::acquire(&root, LaunchClass::RuntimePreview).unwrap();
        fs::write(root.join("ready"), b"ready").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !root.join("stop").exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn native_subprocess_preview_excludes_source_transition() {
        let root = project();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "project_access::tests::subprocess_preview_helper",
            ])
            .env("GODOT_QA_LEASE_TEST_ROOT", &root)
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !root.join("ready").exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let ready = root.join("ready").exists();
        let blocked = ProjectLease::exclusive(&root).is_err();
        fs::write(root.join("stop"), b"stop").unwrap();
        let status = child.wait().unwrap();
        assert!(
            ready && status.success(),
            "child must acquire its preview lease"
        );
        assert!(
            blocked,
            "a different process must exclude source transitions"
        );
        assert!(ProjectLease::exclusive(&root).is_ok());
        fs::remove_dir_all(root).unwrap();
    }
}
