use std::process::Command;

fn qalc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_qalc"))
}

#[test]
fn accepts_space_separated_test_file_argument() {
    let path =
        std::env::temp_dir().join(format!("qalc-{}-space-separated.batch", std::process::id()));
    std::fs::write(&path, "1 + 1\n\t2\n").expect("temporary transcript is writable");

    let output = qalc()
        .arg("--test-file")
        .arg(&path)
        .output()
        .expect("qalc runs");

    let _ = std::fs::remove_file(&path);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("1/1 passed"),
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn rejects_unknown_flags_with_usage() {
    let output = qalc()
        .arg("--definitely-not-a-qalc-option")
        .output()
        .expect("qalc runs");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown option"), "stderr: {stderr}");
    assert!(stderr.contains("Usage: qalc"), "stderr: {stderr}");
}
