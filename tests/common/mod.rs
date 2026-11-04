#![allow(dead_code)] // each test binary uses a different subset
use std::process::Command;

/// Runs a command; returns (success, stdout) and prints full output context on assertion failure.
pub fn run(cmd: &str, args: &[&str]) -> (bool, String) {
    let o = Command::new(cmd).args(args).output().unwrap_or_else(|e| panic!("spawn {cmd}: {e}"));
    let out = String::from_utf8_lossy(&o.stdout).into_owned();
    if !o.status.success() {
        eprintln!("$ {cmd} {args:?}\nstatus: {}\nstdout: {out}\nstderr: {}", o.status, String::from_utf8_lossy(&o.stderr));
    }
    (o.status.success(), out)
}

pub fn ok(cmd: &str, args: &[&str]) -> String {
    let (success, out) = run(cmd, args);
    assert!(success, "`{cmd} {}` failed", args.join(" "));
    out
}

pub fn fails(cmd: &str, args: &[&str]) {
    assert!(!run(cmd, args).0, "`{cmd} {}` unexpectedly succeeded", args.join(" "));
}
