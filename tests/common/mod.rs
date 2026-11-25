#![allow(dead_code)] // each test binary uses a different subset
use std::process::Command;

pub mod slurm;

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

/// Replaces a file via rename, so daemons reading it per lookup never see a truncated copy.
pub fn write_atomic(path: &str, content: impl AsRef<[u8]>) -> std::io::Result<()> {
    let tmp = format!("{path}.tmp-{}", std::process::id());
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, path)
}

const CONF: &str = "/etc/proposal-nss.conf";

/// Appends `line` to /etc/proposal-nss.conf for the guard's lifetime, then restores the file.
pub struct ConfLine(String);

impl ConfLine {
    pub fn add(line: &str) -> Self {
        let old = std::fs::read_to_string(CONF).unwrap();
        write_atomic(CONF, format!("{old}\n{line}\n")).unwrap();
        Self(old)
    }
}

impl Drop for ConfLine {
    fn drop(&mut self) {
        write_atomic(CONF, &self.0).unwrap();
    }
}

pub fn require_prop_dir() -> ConfLine {
    ConfLine::add("require_prop_dir = true")
}
