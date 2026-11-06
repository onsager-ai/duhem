//! Exercise the consumer-pins entry point through the built binary.
use std::process::Command;

fn xtask() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_xtask"));
    command.env("CARGO_MANIFEST_DIR", env!("CARGO_MANIFEST_DIR"));
    command
}

#[test]
fn consumer_pin_entry_point_accepts_a_pin_and_rejects_invalid_input() {
    let output = xtask()
        .args(["consumer-pins", "--pinned", "0.5.1"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("::warning::consumer-pins"));
    for args in [
        vec!["consumer-pins"],
        vec!["consumer-pins", "--pinned", "latest"],
    ] {
        assert!(!xtask().args(args).output().unwrap().status.success());
    }
}
