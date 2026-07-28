//! Cross-platform termination-signal wait used by the run lifecycle.

#[cfg(unix)]
pub(crate) async fn termination_signal() -> &'static str {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, OnceLock};
    use std::time::Duration;

    static FLAGS: OnceLock<(Arc<AtomicBool>, Arc<AtomicBool>)> = OnceLock::new();
    let (sigint, sigterm) = FLAGS.get_or_init(|| {
        use signal_hook::consts::signal::{SIGINT, SIGTERM};

        let sigint = Arc::new(AtomicBool::new(false));
        let sigterm = Arc::new(AtomicBool::new(false));
        signal_hook::flag::register(SIGINT, sigint.clone()).expect("install SIGINT handler");
        signal_hook::flag::register(SIGTERM, sigterm.clone()).expect("install SIGTERM handler");
        (sigint, sigterm)
    });

    loop {
        if sigint.swap(false, Ordering::SeqCst) {
            return "SIGINT";
        }
        if sigterm.swap(false, Ordering::SeqCst) {
            return "SIGTERM";
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[cfg(not(unix))]
pub(crate) async fn termination_signal() -> &'static str {
    tokio::signal::ctrl_c()
        .await
        .expect("install Ctrl-C handler");
    "SIGINT"
}
