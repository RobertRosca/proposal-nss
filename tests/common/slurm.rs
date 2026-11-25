// Helpers shared by the slurm and slurm_conf test binaries.
use super::ok;
use std::ffi::OsStr;
use std::fs;

/// Polls `f` once a second for up to `secs` seconds; true as soon as it returns true.
pub fn wait(secs: u32, mut f: impl FnMut() -> bool) -> bool {
    (0..secs).any(|_| f() || {
        std::thread::sleep(std::time::Duration::from_secs(1));
        false
    })
}

/// Submits `--wrap cmd` as `who` to `partition`; returns (accepted, job output or sbatch stderr).
pub fn job(name: &str, who: &[impl AsRef<OsStr>], partition: &str, cmd: &str) -> (bool, String) {
    let out = format!("/tmp/slurm-{name}.out");
    fs::write(&out, "").unwrap();
    ok("chmod", &["666", &out]);
    let o = std::process::Command::new("sbatch")
        .args(["--wait", "--chdir", "/tmp", "--partition", partition, "--output", &out])
        .args(who)
        .args(["--wrap", cmd])
        .output()
        .unwrap();
    let text = if o.status.success() { fs::read_to_string(&out).unwrap() } else { String::from_utf8_lossy(&o.stderr).into_owned() };
    (o.status.success(), text)
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
