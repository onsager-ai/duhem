//! Live-run on-ramp (#298): point the operator at the dashboard's
//! live page for the run that is about to execute.
//!
//! The dashboard already streams a run's evidence mid-flight
//! (`/api/runs/<id>/live`); what was missing is the way *in*. When a
//! dashboard base is resolvable, `duhem run` mints the run id up
//! front, pins the engine to it, and prints the deep link on stderr
//! before the run starts — stdout stays byte-identical for machine
//! reporters and CI scripts.
//!
//! Resolution ladder:
//! 1. `DUHEM_DASHBOARD_URL` — explicit operator intent (a remote or
//!    reverse-proxied dashboard); trusted without probing.
//! 2. `dashboard.addr` next to the evidence DB — written by a serving
//!    dashboard on bind (`duhem_evidence::dashboard_addr_path`),
//!    removed on shutdown. Probed with a short TCP connect so a stale
//!    file after a crash stays silent.
//!
//! No base → no output. The on-ramp never affects the run itself.

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

pub const URL_ENV_OVERRIDE: &str = "DUHEM_DASHBOARD_URL";

/// Env override for the `--watch` opener binary (#305) — the test
/// seam, and an escape hatch for exotic desktops. The URL is passed
/// as the single argument.
pub const OPENER_ENV_OVERRIDE: &str = "DUHEM_OPENER";

/// How long a `dashboard.addr` liveness probe may take. Local
/// loopback connects resolve in microseconds; this only bounds the
/// pathological case (firewalled port, half-dead peer).
const PROBE_TIMEOUT: Duration = Duration::from_millis(250);

/// The dashboard base URL for the store at `db_path`, if one is
/// resolvable right now. See the module docs for the ladder.
pub fn resolve_dashboard_base(db_path: &Path) -> Option<String> {
    if let Ok(url) = std::env::var(URL_ENV_OVERRIDE)
        && !url.trim().is_empty()
    {
        return Some(url.trim().trim_end_matches('/').to_string());
    }
    let advertised = std::fs::read_to_string(duhem_evidence::dashboard_addr_path(db_path)).ok()?;
    let base = advertised.trim().trim_end_matches('/');
    if base.is_empty() || !probe(base) {
        return None;
    }
    Some(base.to_string())
}

/// The SPA's hash-routed run deep link (#86: hash routing so the same
/// link works on a static export).
pub fn run_page_url(base: &str, run_id: &str) -> String {
    format!("{base}/#/run/{run_id}")
}

/// Open `url` in the operator's browser, best-effort (`--watch`,
/// #305). Detached and silent: a missing or failing opener must
/// never disturb the run.
pub fn open_in_browser(url: &str) {
    let (program, args) = opener_command();
    let _ = std::process::Command::new(program)
        .args(args)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// The platform opener, unless `DUHEM_OPENER` overrides it.
fn opener_command() -> (String, Vec<String>) {
    if let Ok(o) = std::env::var(OPENER_ENV_OVERRIDE)
        && !o.trim().is_empty()
    {
        return (o.trim().to_string(), Vec::new());
    }
    if cfg!(target_os = "macos") {
        ("open".to_string(), Vec::new())
    } else if cfg!(target_os = "windows") {
        // `start` is a cmd built-in; the empty string fills the
        // window-title slot so the URL isn't consumed as the title.
        (
            "cmd".to_string(),
            vec!["/c".to_string(), "start".to_string(), String::new()],
        )
    } else {
        ("xdg-open".to_string(), Vec::new())
    }
}

/// `true` iff something is listening at the advertised base. The file
/// is written by a live dashboard and removed on shutdown, so this
/// only defuses the crash-leftover case; a false positive (port
/// reused by another process) merely prints a dead link.
fn probe(base: &str) -> bool {
    let Some(addr) = base
        .strip_prefix("http://")
        .and_then(|rest| rest.parse::<SocketAddr>().ok())
    else {
        return false;
    };
    std::net::TcpStream::connect_timeout(&addr, PROBE_TIMEOUT).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Env-var tests set/remove `DUHEM_DASHBOARD_URL`, which is
    // process-global — serialize them so parallel test threads don't
    // race each other's env state.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_env<R>(value: Option<&str>, f: impl FnOnce() -> R) -> R {
        let _guard = ENV_LOCK.lock().unwrap();
        // SAFETY: guarded by ENV_LOCK; no other test mutates this var.
        unsafe {
            match value {
                Some(v) => std::env::set_var(URL_ENV_OVERRIDE, v),
                None => std::env::remove_var(URL_ENV_OVERRIDE),
            }
        }
        let out = f();
        unsafe { std::env::remove_var(URL_ENV_OVERRIDE) };
        out
    }

    #[test]
    fn env_override_wins_and_is_normalized() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("duhem.db");
        let base = with_env(Some("http://dash.example:9999/"), || {
            resolve_dashboard_base(&db)
        });
        assert_eq!(base.as_deref(), Some("http://dash.example:9999"));
    }

    #[test]
    fn no_env_no_addr_file_is_silent() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("duhem.db");
        let base = with_env(None, || resolve_dashboard_base(&db));
        assert_eq!(base, None);
    }

    #[test]
    fn live_addr_file_resolves_and_dead_one_stays_silent() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("duhem.db");
        let addr_file = duhem_evidence::dashboard_addr_path(&db);

        // A real listener → the advertised base resolves.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::fs::write(&addr_file, format!("http://{addr}\n")).unwrap();
        let base = with_env(None, || resolve_dashboard_base(&db));
        assert_eq!(base, Some(format!("http://{addr}")));

        // Listener gone (crash leftover) → the probe defuses the file.
        drop(listener);
        let base = with_env(None, || resolve_dashboard_base(&db));
        assert_eq!(base, None);
    }

    #[test]
    fn run_page_url_is_the_hash_routed_deep_link() {
        assert_eq!(
            run_page_url("http://127.0.0.1:7878", "01HXYZ"),
            "http://127.0.0.1:7878/#/run/01HXYZ"
        );
    }

    /// #305: `DUHEM_OPENER` replaces the platform opener wholesale —
    /// the test seam `--watch` integration tests rely on.
    #[test]
    fn opener_env_override_wins_over_platform_default() {
        let _guard = ENV_LOCK.lock().unwrap();
        // SAFETY: guarded by ENV_LOCK; no other test mutates this var.
        unsafe { std::env::set_var(OPENER_ENV_OVERRIDE, "/usr/bin/false") };
        let (program, args) = opener_command();
        unsafe { std::env::remove_var(OPENER_ENV_OVERRIDE) };
        assert_eq!(program, "/usr/bin/false");
        assert!(args.is_empty());
    }
}
