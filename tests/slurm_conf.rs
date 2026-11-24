// Needs a running single-node Slurm; see Dockerfile and .config/nextest.toml.
mod common;
use common::slurm::{assert_lookup_identical, clear, exp_fixture, job, log, node_state, prop, uid, wait, EXP};
use common::{ok, require_prop_dir};
use std::fs;

// Tests that switch the module for every daemon via require_prop_dir() (the fixture conf turns it
// off) for the guard's lifetime. The single container is head and compute node at once, so every
// daemon sees the setting; this binary runs exclusively (`threads-required` in .config/nextest.toml)
// because proposal users without a dir vanish. Conf-changing slurm tests belong here, not in slurm.rs.

/// With require_prop_dir the proposal user does not resolve without its dir: sbatch rejects the job
/// at submit time and the prolog never runs, so no junk /nonexistent.
#[test]
fn require_prop_dir_rejects_job_for_proposal_without_dir() {
    let _g = require_prop_dir();
    let _ = fs::remove_dir_all("/nonexistent");
    clear(8);
    let (accepted, err) = job("req-nodir", &prop(1238), "debug", "true");
    assert!(!accepted && err.contains("Invalid --uid specification"), "{err}");
    assert!(!std::path::Path::new("/nonexistent").exists());
    assert!(log("prolog", 8).is_empty(), "{}", log("prolog", 8));
    assert_eq!(node_state(), "idle");
}

#[test]
fn require_prop_dir_accepts_job_for_proposal_with_dir() {
    let _g = require_prop_dir();
    exp_fixture();
    let home = format!("{EXP}/scratch/.exfl_prop_syn-home");
    let _ = fs::remove_dir_all(&home);
    clear(7);
    let (accepted, out) = job("req-dir", &prop(1237), "debug", "echo ran as $(id -un)");
    assert!(accepted && out.contains("ran as euxfel-p001237"), "{out}");
    assert_eq!(node_state(), "idle", "{}", log("prolog", 7));
    assert!(std::path::Path::new(&home).is_dir(), "{home} missing");
}

#[test]
fn require_prop_dir_lookup_identical_for_root_slurm_and_non_member() {
    let _g = require_prop_dir();
    assert_lookup_identical();
}

fn job_field(id: &str, key: &str) -> String {
    let out = ok("scontrol", &["show", "job", id]);
    out.split_whitespace().find_map(|w| w.strip_prefix(&format!("{key}="))).unwrap_or_default().to_string()
}

/// The dir disappears while the job is queued (GPFS gone, proposal moved): slurmd cannot resolve
/// the user at launch, the job is requeued once and held (launch_failed_requeued_held); there is no
/// requeue loop and the node stays idle. Once the dir is back, `scontrol release` plus
/// `scontrol update StartTime=now` (the requeue also sets a BeginTime delay) runs the job.
#[test]
fn require_prop_dir_holds_job_when_dir_vanishes_before_launch() {
    exp_fixture();
    clear(7);
    // select/linear: the blocker owns the node, so the proposal job stays pending until it ends
    ok("sbatch", &["--uid", "55636", "--gid", "3555", "--chdir", "/tmp", "--output", "/dev/null", "--wrap", "sleep 8"]);
    let out = "/tmp/slurm-req-held.out";
    fs::write(out, "").unwrap();
    ok("chmod", &["666", out]);
    let id = ok("sbatch", &["--parsable", "--chdir", "/tmp", "--output", out, "--uid", &uid(1237), "--gid", "15355", "--wrap", "echo ran as $(id -un)"]);
    let id = id.trim().to_string();

    let g = require_prop_dir();
    fs::remove_dir_all("/fake-exp/FXE/202201/p001237").unwrap();
    let mut reason = String::new();
    wait(60, || {
        reason = job_field(&id, "Reason");
        reason == "launch_failed_requeued_held"
    });
    assert_eq!(reason, "launch_failed_requeued_held", "slurmd log:\n{}", fs::read_to_string("/var/log/slurm/slurmd.log").unwrap_or_default());
    assert_eq!(job_field(&id, "JobState"), "PENDING");
    assert_eq!(job_field(&id, "Restarts"), "1");
    assert_eq!(node_state(), "idle");

    // recovery: dir and config back, release the hold
    drop(g);
    exp_fixture();
    // slurmd remembers the failed launch of this job id for a while ("already killed, do not
    // launch batch job"): a release straight away fails the job with exit code 1; 25 s worked, 0 s did not
    std::thread::sleep(std::time::Duration::from_secs(30));
    ok("scontrol", &["release", &id]);
    // the launch-failure requeue also defers the job (Reason=BeginTime); start it now
    ok("scontrol", &["update", &format!("JobId={id}"), "StartTime=now"]);
    let mut state = String::new();
    wait(60, || {
        state = job_field(&id, "JobState");
        state == "COMPLETED"
    });
    assert_eq!(state, "COMPLETED", "{}", job_field(&id, "Reason"));
    assert!(fs::read_to_string(out).unwrap().contains("ran as euxfel-p001237"));
    assert_eq!(node_state(), "idle");
}
