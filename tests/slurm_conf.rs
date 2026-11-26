// Needs a running single-node Slurm; see Dockerfile and .config/nextest.toml.
mod common;
use common::slurm::{assert_lookup_identical, clear, exp_fixture, job, job_field, log, node_state, prop, uid, wait, EXP};
use common::{ok, require_prop_dir};
use std::fs;

// Tests that switch the module for every daemon via require_prop_dir() (the fixture conf turns it
// off) for the guard's lifetime. The single container is head and compute node at once, so every
// daemon sees the setting; this binary runs exclusively (`threads-required` in .config/nextest.toml)
// because proposal users without a dir vanish. Conf-changing slurm tests belong here, not in slurm.rs.

/// With require_prop_dir the proposal user does not resolve without its dir: sbatch rejects the job
/// at submit time and the prolog never runs, so no fallback home is created.
#[test]
fn require_prop_dir_rejects_job_for_proposal_without_dir() {
    let _g = require_prop_dir();
    let _ = fs::remove_dir_all("/tmp/euxfel-p001238");
    clear(8);
    let (accepted, err) = job("req-nodir", &prop(1238), "debug", "true");
    assert!(!accepted && err.contains("Invalid --uid specification"), "{err}");
    assert!(!std::path::Path::new("/tmp/euxfel-p001238").exists());
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

/// The dir disappears while the job is queued (GPFS gone, proposal moved): slurmd cannot resolve
/// the user at launch, the job is requeued once and held (launch_failed_requeued_held); there is no
/// requeue loop and the node stays idle. Once the dir is back, `scontrol release` plus
/// `scontrol update StartTime=now` (the requeue also sets a BeginTime delay) runs the job.
#[test]
fn require_prop_dir_holds_job_when_dir_vanishes_before_launch() {
    exp_fixture();
    clear(7);
    // select/linear: the blocker owns the node, so the proposal job stays pending until it is cancelled
    let blocker = ok("sbatch", &["--parsable", "--uid", "55636", "--gid", "3555", "--chdir", "/tmp", "--output", "/dev/null", "--wrap", "sleep 300"]);
    let out = "/tmp/slurm-req-held.out";
    fs::write(out, "").unwrap();
    ok("chmod", &["666", out]);
    let id = ok("sbatch", &["--parsable", "--chdir", "/tmp", "--output", out, "--uid", &uid(1237), "--gid", "15355", "--wrap", "echo ran as $(id -un)"]);
    let id = id.trim().to_string();

    let g = require_prop_dir();
    fs::remove_dir_all("/fake-exp/FXE/202201/p001237").unwrap();
    ok("scancel", &[blocker.trim()]);
    // the reason is set before the requeue finishes (job still COMPLETING), so wait for both
    let (mut reason, mut state) = (String::new(), String::new());
    wait(60, || {
        reason = job_field(&id, "Reason");
        state = job_field(&id, "JobState");
        reason == "launch_failed_requeued_held" && state == "PENDING"
    });
    assert_eq!(reason, "launch_failed_requeued_held", "slurmd log:\n{}", fs::read_to_string("/var/log/slurm/slurmd.log").unwrap_or_default());
    assert_eq!(state, "PENDING");
    assert_eq!(job_field(&id, "Restarts"), "1");
    assert_eq!(node_state(), "idle");

    // recovery: dir and config back, release the hold
    drop(g);
    exp_fixture();
    // slurmd keeps the failed launch's revoked credential for CredExpire (120 s). On 24.11 that made an
    // immediate release fail ("already killed, do not launch batch job"); 25.11 gives each launch a new
    // SLUID and matches revocations by SLUID (slurmd cred_context.c), so the release can follow at once.
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
