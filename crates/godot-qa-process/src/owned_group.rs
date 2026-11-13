use std::io;
use std::thread;
use std::time::{Duration, Instant};

/// Clean up a child launched with setsid(), whose launch PID is its owned group ID.
/// Return whether SIGKILL was needed; never fall back to signalling a reused PID.
pub(crate) fn terminate(group: i64) -> io::Result<bool> {
    terminate_with(group, is_running)
}

fn terminate_with(
    group: i64,
    mut inspect: impl FnMut(i64) -> io::Result<bool>,
) -> io::Result<bool> {
    let mut inspection_error = match inspect(group) {
        Ok(false) => return Ok(false),
        Ok(true) => None,
        Err(error) => Some(error),
    };
    // This group belongs to our child. Unreadable state must not prevent
    // bounded TERM/KILL attempts, but cannot confirm successful cleanup.
    for (signal, killed) in [(libc::SIGTERM, false), (libc::SIGKILL, true)] {
        if unsafe { libc::kill(-(group as libc::pid_t), signal) } != 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error);
            }
        }
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            match inspect(group) {
                Ok(false) => return inspection_error.map_or(Ok(killed), Err),
                Ok(true) => {}
                Err(error) => {
                    inspection_error.get_or_insert(error);
                }
            }
            if Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
    Err(inspection_error.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::TimedOut,
            format!(
                "owned process group {group} still has running members after TERM/KILL cleanup"
            ),
        )
    }))
}

#[cfg(target_os = "linux")]
fn is_running(group: i64) -> io::Result<bool> {
    if unsafe { libc::kill(-(group as libc::pid_t), 0) } != 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(false);
        }
        return Err(error);
    }
    // kill(-group, 0) also sees orphaned zombies, which cannot be signalled or
    // reaped by this parent. Only running members require further cleanup.
    for entry in std::fs::read_dir("/proc")? {
        let entry = entry?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<i32>().ok())
        else {
            continue;
        };
        let stat = match std::fs::read(entry.path().join("stat")) {
            Ok(stat) => stat,
            Err(error) => {
                if unsafe { libc::getpgid(pid) } == group as libc::pid_t {
                    return Err(error);
                }
                continue;
            }
        };
        let mut fields = crate::proc_stat::fields(&stat).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid /proc/{pid}/stat"),
            )
        })?;
        let state = fields.next();
        let process_group = fields.nth(1).and_then(|field| field.parse::<i64>().ok());
        if process_group == Some(group) && !matches!(state, Some("Z" | "X")) {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(not(target_os = "linux"))]
fn is_running(group: i64) -> io::Result<bool> {
    if unsafe { libc::kill(-(group as libc::pid_t), 0) } == 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(false)
    } else {
        Err(error)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::os::unix::process::{CommandExt, ExitStatusExt};
    use std::process::{Command, Stdio};

    #[test]
    fn inspection_error_still_kills_owned_group() {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "trap '' TERM; printf ready; exec sleep 30"])
            .process_group(0)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut ready = [0; 5];
        child.stdout.take().unwrap().read_exact(&mut ready).unwrap();
        assert_eq!(&ready, b"ready");
        let started = std::time::Instant::now();
        let result = super::terminate_with(i64::from(child.id()), |_| {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "fixture group inspection denied",
            ))
        });
        let exited = child.try_wait().unwrap();
        // Clean up even when the original early-return bug leaves the child alive.
        let _ = child.kill();
        child.wait().unwrap();
        assert!(exited.is_some(), "inspection failure skipped group signals");
        assert_eq!(exited.unwrap().signal(), Some(libc::SIGKILL));
        let error = result.expect_err("unconfirmed group state must remain a cleanup failure");
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        assert!(
            error
                .to_string()
                .contains("fixture group inspection denied")
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(3));
    }
}
