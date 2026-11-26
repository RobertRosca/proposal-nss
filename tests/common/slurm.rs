// Helpers shared by the slurm and slurm_conf test binaries.
use super::ok;
use std::ffi::OsStr;
use std::fs;
use std::time::{Duration, Instant};

/// Polls `f` every 100 ms for up to `secs` seconds; true as soon as it returns true.
pub fn wait(secs: u32, mut f: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(secs.into());
    loop {
        if f() {
            return true;
        }
        if Instant::now() >= end {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

pub fn job_field(id: &str, key: &str) -> String {
    let out = ok("scontrol", &["show", "job", id]);
    out.split_whitespace().find_map(|w| w.strip_prefix(&format!("{key}="))).unwrap_or_default().to_string()
}

/// Polls the job record until a terminal JobState; returns (JobState, ExitCode). Replaces
/// `sbatch --wait`, whose poll backoff (2 s, then x4) quantised test times. scontrol shows
/// COMPLETING until the epilog ends, so this also waits for the epilog. MinJobAge=90 keeps the record.
pub fn wait_job(id: &str) -> (String, String) {
    const DONE: [&str; 9] = ["COMPLETED", "FAILED", "CANCELLED", "TIMEOUT", "NODE_FAIL", "PREEMPTED", "BOOT_FAIL", "DEADLINE", "OUT_OF_MEMORY"];
    let mut state = String::new();
    // generous: in the slurm binary a job can queue behind the QOS test's jobs (select/linear, one node)
    let done = wait(300, || {
        state = job_field(id, "JobState");
        DONE.contains(&state.as_str())
    });
    assert!(done, "job {id} still {state}");
    (state, job_field(id, "ExitCode"))
}

/// Submits `--wrap cmd` as `who` to `partition`; returns (accepted and exited 0, job output or sbatch stderr).
pub fn job(name: &str, who: &[impl AsRef<OsStr>], partition: &str, cmd: &str) -> (bool, String) {
    let out = format!("/tmp/slurm-{name}.out");
    fs::write(&out, "").unwrap();
    ok("chmod", &["666", &out]);
    let o = std::process::Command::new("sbatch")
        .args(["--parsable", "--chdir", "/tmp", "--partition", partition, "--output", &out])
        .args(who)
        .args(["--wrap", cmd])
        .output()
        .unwrap();
    if !o.status.success() {
        return (false, String::from_utf8_lossy(&o.stderr).into_owned());
    }
    let (state, exit) = wait_job(String::from_utf8_lossy(&o.stdout).trim());
    let text = fs::read_to_string(&out).unwrap();
    if state == "COMPLETED" && exit == "0:0" { (true, text) } else { (false, format!("JobState={state} ExitCode={exit}\n{text}")) }
}

/// uid of proposal `no` (see src/lib.rs: `60{no:06}2`); proposals 1241-1244 have fixture groups.
pub fn uid(no: u32) -> String {
    format!("60{no:06}2")
}

pub fn prop(no: u32) -> Vec<String> {
    vec!["--uid".into(), uid(no), "--gid".into(), "15355".into()]
}

pub const EXP: &str = "/fake-exp/FXE/202201/p001237";
pub const SCRATCH: &str = "/fake-scratch/FXE/202201/p001237";

/// Production layout of p001237 (POSIX approximation of the NFSv4 ACLs): p{no} is 0550 xdata:xdata,
/// so only root and xdata can enter it; scratch is a symlink to a tree whose dir belongs to the
/// proposal group (rwx for 60001237-part only). Idempotent.
pub fn exp_fixture() {
    fs::create_dir_all(SCRATCH).unwrap();
    ok("chgrp", &["60001237-part", SCRATCH]);
    ok("chmod", &["770", SCRATCH]);
    fs::create_dir_all(EXP).unwrap();
    let link = format!("{EXP}/scratch");
    if fs::symlink_metadata(&link).is_err() {
        std::os::unix::fs::symlink(SCRATCH, &link).unwrap();
    }
    ok("chown", &["xdata:xdata", "/fake-exp/FXE/202201", EXP]);
    ok("chmod", &["550", EXP]);
}

/// The module only needs to traverse the cycle dir to see p{no}: lookups must not depend on the
/// caller being able to enter the 0550 xdata:xdata p{no} (slurmctld runs as `slurm`, users are non-members).
pub fn assert_lookup_identical() {
    exp_fixture();
    for key in ["euxfel-p001237", "600012372"] {
        let want = ok("getent", &["passwd", key]);
        assert!(want.contains(&format!(":{EXP}/scratch/.exfl_prop_syn-home:")), "{want}");
        for user in ["slurm", "outsider"] {
            assert_eq!(ok("runuser", &["-u", user, "--", "getent", "passwd", key]), want, "{key} as {user}");
        }
    }
}

/// Trace of the prolog/epilog for `euxfel-p00123{n}` (n=7 or 8); `clear` it before the job under test.
pub fn log(f: &str, n: u8) -> String {
    fs::read_to_string(format!("/var/log/slurm/{f}-test.euxfel-p00123{n}.log")).unwrap_or_default()
}

pub fn clear(n: u8) {
    for f in ["prolog", "epilog"] {
        let _ = fs::remove_file(format!("/var/log/slurm/{f}-test.euxfel-p00123{n}.log"));
    }
}

/// The epilog finishes after the job; the node must settle on idle, never drain.
pub fn node_state() -> String {
    let mut s = String::new();
    wait(30, || {
        let o = ok("sinfo", &["-h", "-N", "-o", "%T"]); // one line per partition
        s = o.lines().next().unwrap().to_string();
        assert!(o.lines().all(|l| l == s), "{o}");
        s == "idle" || s.starts_with("drain")
    });
    s
}
