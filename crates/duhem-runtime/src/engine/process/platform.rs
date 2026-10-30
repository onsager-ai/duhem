//! Unix process-group ownership. Keep the leader unreaped while observing it,
//! pinning its PID/PGID until every live group member has terminated.
use std::io;

#[derive(Debug, Clone, Copy)]
pub(super) struct Exit {
    pub code: Option<i32>,
    pub signal: Option<i32>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn observe(pid: u32) -> io::Result<Option<Exit>> {
    // SAFETY: waitid initializes this zeroed siginfo; WNOWAIT deliberately
    // does not reap. Only our retained Child is reaped by stop().
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let rc = unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { info.si_pid() } == 0 {
        return Ok(None);
    }
    let status = unsafe { info.si_status() };
    Ok(Some(if info.si_code == libc::CLD_EXITED {
        Exit {
            code: Some(status),
            signal: None,
        }
    } else {
        Exit {
            code: None,
            signal: Some(status),
        }
    }))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn signal(group: u32, signal: i32) -> io::Result<()> {
    // SAFETY: group is the PID of our unreaped child, never authored input.
    if unsafe { libc::kill(-(group as i32), signal) } == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(error)
    }
}

#[cfg(target_os = "linux")]
pub(super) fn group_live(group: u32) -> io::Result<bool> {
    for entry in std::fs::read_dir("/proc")? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().parse::<u32>().is_err() {
            continue;
        }
        let stat = match std::fs::read_to_string(entry.path().join("stat")) {
            Ok(stat) => stat,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        // comm is parenthesized and may itself contain spaces or ')'.
        let fields: Vec<_> = stat
            .rsplit_once(')')
            .ok_or_else(|| io::Error::other("invalid /proc stat"))?
            .1
            .split_whitespace()
            .take(3)
            .collect();
        if fields.len() != 3 {
            return Err(io::Error::other("short /proc stat"));
        }
        if fields[2].parse::<u32>().ok() == Some(group) && !matches!(fields[0], "Z" | "X") {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(target_os = "macos")]
pub(super) fn group_live(group: u32) -> io::Result<bool> {
    use std::mem::{size_of, zeroed};
    use std::ptr::null_mut;
    // libproc reports group membership without reaping the pinned leader.
    // Unlike proc_listpids, this convenience function returns a PID count,
    // while its buffer capacity is still expressed in bytes.
    let count = unsafe { libc::proc_listpgrppids(group as i32, null_mut(), 0) };
    if count < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut pids = vec![0i32; count as usize + 64];
    let capacity = (pids.len() * size_of::<i32>()) as i32;
    let used = unsafe { libc::proc_listpgrppids(group as i32, pids.as_mut_ptr().cast(), capacity) };
    if used < 0 {
        return Err(io::Error::last_os_error());
    }
    if used as usize >= pids.len() {
        return Err(io::Error::other("process group outgrew observation buffer"));
    }
    for pid in pids.into_iter().take(used as usize).filter(|pid| *pid > 0) {
        let mut info: libc::proc_bsdinfo = unsafe { zeroed() };
        let size = size_of::<libc::proc_bsdinfo>() as i32;
        let count = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                (&mut info as *mut libc::proc_bsdinfo).cast(),
                size,
            )
        };
        if count != size {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                continue;
            }
            return Err(error);
        }
        if info.pbi_pgid == group && info.pbi_status != libc::SZOMB {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) fn observe(_: u32) -> io::Result<Option<Exit>> {
    Err(unsupported())
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) fn signal(_: u32, _: i32) -> io::Result<()> {
    Err(unsupported())
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) fn group_live(_: u32) -> io::Result<bool> {
    Err(unsupported())
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn unsupported() -> io::Error {
    io::Error::other("owned processes require Linux or macOS")
}

pub(super) const TERM: i32 = 15;
pub(super) const KILL: i32 = 9;
