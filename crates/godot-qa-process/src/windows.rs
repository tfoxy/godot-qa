//! Windows process ownership. Handles pin a process identity even if its PID is reused.
use std::io;
use std::mem::{size_of, zeroed};
use std::os::windows::io::AsRawHandle;
use std::process::Child;
use windows_sys::Win32::Foundation::{
    CloseHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Threading::{
    GetProcessIdOfThread, GetProcessTimes, OpenProcess, OpenThread,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, ResumeThread,
    SuspendThread, THREAD_QUERY_LIMITED_INFORMATION, THREAD_SUSPEND_RESUME, TerminateProcess,
    WaitForSingleObject,
};

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn open(pid: i64, access: u32) -> io::Result<Handle> {
    let pid = u32::try_from(pid)
        .ok()
        .filter(|pid| *pid > 0)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid process id"))?;
    let handle = unsafe { OpenProcess(access, 0, pid) };
    if handle.is_null() {
        Err(io::Error::last_os_error())
    } else {
        Ok(Handle(handle))
    }
}
fn created(handle: HANDLE) -> io::Result<u64> {
    let mut creation: FILETIME = unsafe { zeroed() };
    let mut exit = creation;
    let mut kernel = creation;
    let mut user = creation;
    if unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime))
}
pub(crate) fn identity(pid: i64) -> Option<u64> {
    created(open(pid, PROCESS_QUERY_LIMITED_INFORMATION).ok()?.0).ok()
}
pub(crate) fn is_running(pid: i64) -> bool {
    match open(pid, PROCESS_SYNCHRONIZE) {
        Ok(handle) => unsafe { WaitForSingleObject(handle.0, 0) == WAIT_TIMEOUT },
        // An inaccessible process is still alive. Never treat access-denied as permission
        // to prune its artifacts, or as proof that cleanup succeeded.
        Err(error) => error.raw_os_error() == Some(5),
    }
}

/// Freeze and terminate the owned tree. Open handles before killing anything and
/// compare creation times so reused parent/child PIDs cannot target unrelated processes.
pub(crate) fn terminate_tree(pid: i64, expected: Option<u64>) -> io::Result<()> {
    let root = match open(
        pid,
        PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
    ) {
        Ok(root) => root,
        Err(error) if error.raw_os_error() == Some(87) => return Ok(()),
        Err(error) => return Err(error),
    };
    let root_creation = created(root.0)?;
    if expected.is_some_and(|expected| expected != root_creation) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "process creation identity no longer matches the owned session",
        ));
    }
    let mut tree = vec![(pid as u32, root_creation, root)];
    let mut suspended = Vec::new();
    let mut index = 0;
    while index < tree.len() {
        let (parent, parent_creation, process) = &tree[index];
        let (parent, parent_creation) = (*parent, *parent_creation);
        // Freeze each parent before discovering its children. This prevents a wrapper
        // from creating new descendants after our snapshot. Failure resumes every thread.
        suspended.extend(freeze_threads(parent, process.0)?);
        let entries = process_snapshot()?;
        for &(child, owner) in &entries {
            if owner != parent || tree.iter().any(|(seen, _, _)| *seen == child) {
                continue;
            }
            let handle = match open(
                i64::from(child),
                PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            ) {
                Ok(handle) => handle,
                Err(error) if error.raw_os_error() == Some(87) => continue,
                Err(error) => return Err(error),
            };
            let creation = created(handle.0)?;
            if creation >= parent_creation {
                tree.push((child, creation, handle));
            }
        }
        index += 1;
    }
    for (_, _, handle) in tree.iter().rev() {
        if unsafe { WaitForSingleObject(handle.0, 0) } != WAIT_TIMEOUT {
            continue;
        }
        if unsafe { TerminateProcess(handle.0, 1) } == 0
            && unsafe { WaitForSingleObject(handle.0, 0) } == WAIT_TIMEOUT
        {
            return Err(io::Error::last_os_error());
        }
    }
    for (_, _, handle) in &tree {
        if unsafe { WaitForSingleObject(handle.0, 5000) } == WAIT_TIMEOUT {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "owned process tree did not exit",
            ));
        }
    }
    Ok(())
}

fn process_snapshot() -> io::Result<Vec<(u32, u32)>> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let snapshot = Handle(snapshot);
    let mut entry: PROCESSENTRY32W = unsafe { zeroed() };
    entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
    let mut entries = Vec::new();
    let mut present = unsafe { Process32FirstW(snapshot.0, &mut entry) } != 0;
    while present {
        entries.push((entry.th32ProcessID, entry.th32ParentProcessID));
        present = unsafe { Process32NextW(snapshot.0, &mut entry) } != 0;
    }
    Ok(entries)
}

struct SuspendedThread(Handle);
impl Drop for SuspendedThread {
    fn drop(&mut self) {
        unsafe {
            ResumeThread(self.0.0);
        }
    }
}

fn freeze_threads(pid: u32, process: HANDLE) -> io::Result<Vec<SuspendedThread>> {
    let mut frozen = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    // A second pass catches threads created between the snapshot and suspension.
    for _ in 0..128 {
        if unsafe { WaitForSingleObject(process, 0) } != WAIT_TIMEOUT {
            return Ok(frozen);
        }
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let snapshot = Handle(snapshot);
        let mut entry: THREADENTRY32 = unsafe { zeroed() };
        entry.dwSize = size_of::<THREADENTRY32>() as u32;
        let mut present = unsafe { Thread32First(snapshot.0, &mut entry) } != 0;
        let mut changed = false;
        while present {
            if entry.th32OwnerProcessID == pid && !seen.contains(&entry.th32ThreadID) {
                let raw = unsafe {
                    OpenThread(
                        THREAD_SUSPEND_RESUME | THREAD_QUERY_LIMITED_INFORMATION,
                        0,
                        entry.th32ThreadID,
                    )
                };
                if raw.is_null() {
                    let error = io::Error::last_os_error();
                    if error.raw_os_error() != Some(87) {
                        return Err(error);
                    }
                } else {
                    let thread = Handle(raw);
                    // Opening a thread pins its identity; validate the snapshot's owner
                    // before suspending, in case the original thread already exited.
                    if unsafe { GetProcessIdOfThread(thread.0) } == pid {
                        if unsafe { SuspendThread(thread.0) } == u32::MAX {
                            if unsafe { WaitForSingleObject(process, 0) } == WAIT_TIMEOUT {
                                return Err(io::Error::last_os_error());
                            }
                        } else {
                            seen.insert(entry.th32ThreadID);
                            frozen.push(SuspendedThread(thread));
                            changed = true;
                        }
                    }
                }
            }
            present = unsafe { Thread32Next(snapshot.0, &mut entry) } != 0;
        }
        if !changed {
            return Ok(frozen);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "process tree kept creating threads during cleanup",
    ))
}

/// A foreground command owns its job until output collection and cleanup finish.
/// Detached sessions deliberately do not use KILL_ON_JOB_CLOSE.
pub(crate) struct OwnedJob(Handle);
impl OwnedJob {
    pub(crate) fn attach(child: &Child) -> io::Result<Self> {
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let handle = Handle(handle);
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if unsafe {
            SetInformationJobObject(
                handle.0,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        } == 0
            || unsafe { AssignProcessToJobObject(handle.0, child.as_raw_handle()) } == 0
        {
            return Err(io::Error::last_os_error());
        }
        // The child was created suspended, so none of its descendants can escape
        // assignment to the job. Resume its initial thread only after ownership exists.
        let threads = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if threads == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let threads = Handle(threads);
        let mut entry: THREADENTRY32 = unsafe { zeroed() };
        entry.dwSize = size_of::<THREADENTRY32>() as u32;
        let mut present = unsafe { Thread32First(threads.0, &mut entry) } != 0;
        while present {
            if entry.th32OwnerProcessID == child.id() {
                let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
                if thread.is_null() {
                    return Err(io::Error::last_os_error());
                }
                let thread = Handle(thread);
                if unsafe { ResumeThread(thread.0) } == u32::MAX {
                    return Err(io::Error::last_os_error());
                }
                return Ok(Self(handle));
            }
            present = unsafe { Thread32Next(threads.0, &mut entry) } != 0;
        }
        Err(io::Error::other(
            "suspended child initial thread was not found",
        ))
    }
    pub(crate) fn terminate(&self) -> io::Result<()> {
        if unsafe { TerminateJobObject(self.0.0, 1) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}
