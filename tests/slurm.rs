// Needs a running single-node Slurm; see test/entrypoint.sh.
mod common;
use common::{ok, run};
use std::fs;

const U: &str = "600012342";
const N: &str = "euxfel-p001234";

#[test]
fn proposal_user_job_submits_nested_job_as_itself() {
    let d = "/tmp/slurm-nested";
    fs::create_dir_all(d).unwrap();
    for f in ["outer.out", "inner.out"] {
        fs::write(format!("{d}/{f}"), "").unwrap();
        ok("chmod", &["666", &format!("{d}/{f}")]);
    }
    ok("chmod", &["777", d]);
    fs::write(
        format!("{d}/outer.sh"),
        format!(
            "#!/bin/bash\nset -e\necho \"outer uid=$(id -u) user=$(id -un)\"\n\
             inner=$(sbatch --parsable --chdir={d} --output={d}/inner.out --wrap 'echo \"inner uid=$(id -u) user=$(id -un)\"')\n\
             scontrol show job \"$inner\" | grep -o 'UserId=[^ ]*'\n"
        ),
    )
    .unwrap();
    ok("chmod", &["755", &format!("{d}/outer.sh")]);

    ok("sbatch", &["--uid", U, "--gid", "15355", "--wait", "--chdir", d, "--output", &format!("{d}/outer.out"), &format!("{d}/outer.sh")]);

    let outer = fs::read_to_string(format!("{d}/outer.out")).unwrap();
    // select/linear gives the outer job the whole node, so the inner job only runs once it ends.
    let mut inner = String::new();
    for _ in 0..30 {
        inner = fs::read_to_string(format!("{d}/inner.out")).unwrap();
        if !inner.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    assert!(outer.contains(&format!("outer uid={U} user={N}")), "{outer}");
    assert!(outer.contains(&format!("UserId={N}({U})")), "{outer}");
    assert!(inner.contains(&format!("inner uid={U} user={N}")), "{inner}");
}

const PROP: &[&str] = &["--uid", U, "--gid", "15355"];
const OTHER: &[&str] = &["--uid", "55636", "--gid", "3555"];// real user, primary group exfel (3555)

/// Submits `--wrap cmd` as `who` to `partition`; returns (accepted, job output or sbatch stderr).
fn job(name: &str, who: &[&str], partition: &str, cmd: &str) -> (bool, String) {
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

const DENIED: &str = "User's group not permitted to use this partition";

// Partition AllowGroups: slurmctld builds the allowed uid set from each group's gr_mem plus a
// getpwent scan for users whose primary gid matches. The module has no enumeration and no
// group entries with members, so proposal users are denied everywhere; observed:
//   sbatch: error: Batch job submission failed: User's group not permitted to use this partition
// Resolution: append exfel_prop_syn to a populated AllowGroups, see
// allowgroups_real_plus_synthetic_group_accepts_proposal_user and docs/deployment.md. The
// single-group cases below stay denied by design.
// GroupUpdateTime=1 in setup-slurm.sh keeps slurmctld's group cache from hiding changes.

#[test]
fn control_open_partition_accepts_proposal_user() {
    assert!(job("open", PROP, "debug", "true").0);
}

#[test]
fn control_real_user_is_accepted_by_group_partitions() {
    // otheruser: primary gid exfel (found by slurmctld's primary-gid scan of passwd)
    assert!(job("other-sup", OTHER, "sup", "true").0);
    // ...but is not in exfel_prop_syn
    let (accepted, err) = job("other-syn", OTHER, "syn", "true");
    assert!(!accepted && err.contains(DENIED), "{err}");
}

#[test]
fn allowgroups_primary_group_denies_proposal_user() {
    let (accepted, err) = job("prop-syn", PROP, "syn", "true");
    assert!(!accepted && err.contains(DENIED), "{err}");
}

#[test]
fn allowgroups_supplementary_group_denies_proposal_user() {
    // exfel is only a supplementary group (via initgroups), which slurmctld never consults
    let (accepted, err) = job("prop-sup", PROP, "sup", "true");
    assert!(!accepted && err.contains(DENIED), "{err}");
}

#[test]
#[ignore = "slurmctld limitation: AllowGroups=exfel_prop_syn alone denies (empty allowed-uid list); append it to a populated AllowGroups instead"]
fn desired_allowgroups_primary_group_accepts_proposal_user() {
    let (accepted, err) = job("want-syn", PROP, "syn", "true");
    assert!(accepted, "{err}");
}

#[test]
#[ignore = "slurmctld limitation: AllowGroups=exfel denies (supplementary groups are never consulted); append exfel_prop_syn instead"]
fn desired_allowgroups_supplementary_group_accepts_proposal_user() {
    let (accepted, err) = job("want-sup", PROP, "sup", "true");
    assert!(accepted, "{err}");
}

/// With LaunchParameters=disable_send_gids slurmd resolves groups itself (initgroups via NSS).
#[test]
fn proposal_user_job_gets_supplementary_groups() {
    let (accepted, out) = job("groups", PROP, "debug", "id -G; getent initgroups euxfel-p001234");
    assert!(accepted, "{out}");
    // primary exfel_prop_syn, supplementary exfel, plus the proposal group 60001234-part
    assert!(out.starts_with("15355 3555 600012342\n"), "{out}");
}

// Accounting: the container runs slurmdbd with AccountingStorageEnforce=
// associations,limits like production. setup-slurm.sh gives otheruser, xdamnprd and euxfel-p001234
// an association in account `exfel`; `outsider` only has account `other`.

/// uid of proposal `no` (see src/lib.rs: `60{no:06}2`); proposals 1240-1244 have fixture groups.
fn uid(no: u32) -> String {
    format!("60{no:06}2")
}

fn prop(no: u32) -> Vec<String> {
    vec!["--uid".into(), uid(no), "--gid".into(), "15355".into()]
}

fn sacctmgr_add_user(no: u32, account: &str) {
    // sacctmgr resolves the synthetic name through the NSS module: this must work in the container
    ok("sacctmgr", &["-i", "add", "user", &format!("name=euxfel-p{no:06}"), &format!("account={account}")]);
}

fn who(v: &[String]) -> Vec<&str> {
    v.iter().map(String::as_str).collect()
}

const NO_ASSOC: &str = "Invalid account or account/partition combination specified";

#[test]
fn accounting_proposal_user_without_association_is_rejected() {
    // observed: sbatch: error: Batch job submission failed: Invalid account or account/partition combination specified
    let (accepted, err) = job("noassoc", &who(&prop(1240)), "debug", "true");
    assert!(!accepted && err.contains(NO_ASSOC), "{err}");
}

#[test]
fn accounting_association_makes_proposal_user_accepted_at_once() {
    let (accepted, _) = job("pre-assoc", &who(&prop(1241)), "debug", "true");
    assert!(!accepted);
    sacctmgr_add_user(1241, "exfel");
    // observed: slurmdbd pushes the new association to slurmctld within ~15 ms, no retry needed
    let (accepted, err) = job("post-assoc", &who(&prop(1241)), "debug", "true");
    assert!(accepted, "{err}");
}

/// Partition `lim` has QOS joblim (MaxJobsPerUser=1, the analogue of upex.joblim): the limit is
/// counted per proposal user, so every proposal user under account exfel gets its own allowance.
#[test]
fn accounting_partition_qos_limit_applies_per_proposal_user() {
    sacctmgr_add_user(1242, "exfel");
    sacctmgr_add_user(1243, "exfel");
    let submit = |no: u32| -> String {
        let o = std::process::Command::new("sbatch")
            .args(["--parsable", "--chdir", "/tmp", "--output", "/dev/null", "--partition", "lim"])
            .args(who(&prop(no)))
            .args(["--wrap", "sleep 300"])
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    let (a1, a2, b1) = (submit(1242), submit(1242), submit(1243));
    // %T state, %r reason
    let state = |id: &str| ok("squeue", &["-h", "-j", id, "-o", "%T %r"]).trim().to_string();
    let mut s = (String::new(), String::new(), String::new());
    for _ in 0..90 {
        s = (state(&a1), state(&a2), state(&b1));
        if s.0.starts_with("RUNNING") && s.2.starts_with("RUNNING") && s.1.contains("QOSMaxJobsPerUserLimit") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    ok("scancel", &[&a1, &a2, &b1]);
    assert!(s.0.starts_with("RUNNING"), "first job of 1242: {s:?}");
    assert!(s.1.starts_with("PENDING") && s.1.contains("QOSMaxJobsPerUserLimit"), "second job of 1242: {s:?}");
    assert!(s.2.starts_with("RUNNING"), "first job of 1243: {s:?}");
}

// AllowAccounts needs no group lookups, only the association.

#[test]
fn allowaccounts_accepts_associated_proposal_user_and_rejects_other_accounts() {
    let (accepted, err) = job("acct-prop", &who(&prop(1234)), "acct", "true");
    assert!(accepted, "{err}");
    let (accepted, err) = job("acct-other", OTHER, "acct", "true");
    assert!(accepted, "{err}");
    // real user whose only association is account `other`
    let (accepted, err) = job("acct-outsider", &["--uid", "55637", "--gid", "3555"], "acct", "true");
    assert!(!accepted, "{err}");
    eprintln!("outsider rejected: {err}");
    // proposal user associated with a different account is rejected the same way
    sacctmgr_add_user(1244, "other");
    let (accepted, err) = job("acct-prop-other", &who(&prop(1244)), "acct", "true");
    assert!(!accepted, "{err}");
}

/// Surprise: AllowGroups=exfel,exfel_prop_syn ACCEPTS the proposal user, although each group alone
/// denies it. Matrix observed in the container (slurm 24.11.5, scontrol update + sbatch):
///   exfel,exfel_prop_syn / exfel_prop_syn,exfel / exfel_prop_syn,root -> proposal user accepted
///   exfel_prop_syn, exfel_prop_syn,exfel_prop_syn, exfel_prop_syn,<unknown group> -> denied
///   exfel,root / 60001234-part,root -> denied
/// Explained by slurmctld source (partition_mgr.c validate_group, same in 24.11.5 and 25.11.5):
/// allow_uids = gr_mem + getpwent scan of users whose primary gid is a listed group. If that list
/// is EMPTY validate_group returns 0 at once; otherwise, as a documented fallback for sites with
/// enumeration disabled (sssd enumerate=False), it checks the user's PRIMARY group name (getgrgid
/// of the passwd gid) against AllowGroups. So the synthetic primary group works only when at least
/// one other listed group resolves to a non-empty member list; supplementary groups are never used.
#[test]
fn allowgroups_real_plus_synthetic_group_accepts_proposal_user() {
    let (accepted, err) = job("prop-both", &who(&prop(1234)), "both", "true");
    assert!(accepted, "{err}");
    assert!(job("other-both", OTHER, "both", "true").0);
}

// Production prolog/epilog (sanitised copies in test/prod-prolog, installed as /etc/slurm/slurm_{prolog,epilog}.sh,
// PrologFlags=Alloc,Contain,Serial). They trace to /var/log/slurm/{prolog,epilog}-test.<user>.log.
// Exp tree (outside /tmp: the epilog rm -rf deletes the user's files there): /fake-exp (exp_dir in /etc/proposal-nss.conf); p001237 exists there, p001238 does not.

const EXP: &str = "/fake-exp/FXE/202201/p001237";
const HAS_DIR: &[&str] = &["--uid", "600012372", "--gid", "15355"];
const NO_DIR: &[&str] = &["--uid", "600012382", "--gid", "15355"];

/// Trace of the prolog/epilog for `euxfel-p00123{n}` (n=7 or 8); `clear` it before the job under test.
fn log(f: &str, n: u8) -> String {
    fs::read_to_string(format!("/var/log/slurm/{f}-test.euxfel-p00123{n}.log")).unwrap_or_default()
}

fn clear(n: u8) {
    for f in ["prolog", "epilog"] {
        let _ = fs::remove_file(format!("/var/log/slurm/{f}-test.euxfel-p00123{n}.log"));
    }
}

/// The epilog finishes after the job; the node must settle on idle, never drain.
fn node_state() -> String {
    let mut s = String::new();
    for _ in 0..30 {
        let o = ok("sinfo", &["-h", "-N", "-o", "%T"]); // one line per partition
        s = o.lines().next().unwrap().to_string();
        assert!(o.lines().all(|l| l == s), "{o}");
        if s == "idle" || s.starts_with("drain") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    s
}

fn wait_gone(path: &str) -> bool {
    for _ in 0..30 {
        if !std::path::Path::new(path).exists() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    false
}

#[test]
fn prolog_runs_for_proposal_user_and_node_stays_idle() {
    fs::create_dir_all(format!("{EXP}/scratch")).unwrap();
    clear(7);
    let (accepted, out) = job("prolog-ok", HAS_DIR, "debug", "echo ran as $(id -un)");
    assert!(accepted && out.contains("ran as euxfel-p001237"), "{out}");
    assert_eq!(node_state(), "idle", "prolog log:\n{}\nepilog log:\n{}", log("prolog", 7), log("epilog", 7));
    let p = log("prolog", 7);
    assert!(p.contains("mkhomedir_helper euxfel-p001237"), "{p}");
    assert!(log("epilog", 7).contains("pkill -KILL -U 600012372"), "{}", log("epilog", 7));
}

/// The prolog checks /home/$SLURM_JOB_USER (never exists for synthetic users), so mkhomedir_helper
/// runs on every job; with the proposal dir present it creates the module's pw_dir.
#[test]
fn prolog_mkhomedir_creates_pw_dir_under_proposal_scratch() {
    fs::create_dir_all(format!("{EXP}/scratch")).unwrap();
    let home = format!("{EXP}/scratch/.exfl_prop_syn-home");
    let _ = fs::remove_dir_all(&home);
    clear(7);
    assert!(job("prolog-home", HAS_DIR, "debug", "true").0);
    assert_eq!(node_state(), "idle");
    assert!(std::path::Path::new(&home).is_dir(), "{home} missing; prolog log:\n{}", log("prolog", 7));
    assert!(!std::path::Path::new("/home/euxfel-p001237").exists());
}

/// Observed: pw_dir=/nonexistent and mkhomedir_helper creates it at the container root (junk dir).
#[test]
fn prolog_mkhomedir_creates_nonexistent_junk_dir_without_proposal_dir() {
    let _ = fs::remove_dir_all("/nonexistent");
    clear(8);
    assert!(job("prolog-junk", NO_DIR, "debug", "true").0);
    assert_eq!(node_state(), "idle", "{}", log("prolog", 8));
    assert!(std::path::Path::new("/nonexistent").exists(), "prolog log:\n{}", log("prolog", 8));
}

/// Switches the module to `require_prop_dir = true` for the guard's lifetime. The single container is
/// head and compute node at once, so every daemon sees the setting; the tests using this are
/// exclusive (`threads-required` in .config/nextest.toml) because proposal users without a dir vanish.
struct RequirePropDir(String);

const CONF: &str = "/etc/proposal-nss.conf";

impl RequirePropDir {
    fn new() -> Self {
        let old = fs::read_to_string(CONF).unwrap();
        fs::write(CONF, format!("{old}\nrequire_prop_dir = true\n")).unwrap();
        Self(old)
    }
}

impl Drop for RequirePropDir {
    fn drop(&mut self) {
        fs::write(CONF, &self.0).unwrap();
    }
}

/// With require_prop_dir the proposal user does not resolve without its dir: sbatch rejects the job
/// at submit time and the prolog never runs, so no junk /nonexistent.
#[test]
fn require_prop_dir_rejects_job_for_proposal_without_dir() {
    let _g = RequirePropDir::new();
    let _ = fs::remove_dir_all("/nonexistent");
    clear(8);
    let (accepted, err) = job("req-nodir", NO_DIR, "debug", "true");
    assert!(!accepted && err.contains("Invalid --uid specification"), "{err}");
    assert!(!std::path::Path::new("/nonexistent").exists());
    assert!(log("prolog", 8).is_empty(), "{}", log("prolog", 8));
    assert_eq!(node_state(), "idle");
}

#[test]
fn require_prop_dir_accepts_job_for_proposal_with_dir() {
    let _g = RequirePropDir::new();
    fs::create_dir_all(format!("{EXP}/scratch")).unwrap();
    let home = format!("{EXP}/scratch/.exfl_prop_syn-home");
    let _ = fs::remove_dir_all(&home);
    clear(7);
    let (accepted, out) = job("req-dir", HAS_DIR, "debug", "echo ran as $(id -un)");
    assert!(accepted && out.contains("ran as euxfel-p001237"), "{out}");
    assert_eq!(node_state(), "idle", "{}", log("prolog", 7));
    assert!(std::path::Path::new(&home).is_dir(), "{home} missing");
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
    fs::create_dir_all(format!("{EXP}/scratch")).unwrap();
    clear(7);
    // select/linear: the blocker owns the node, so the proposal job stays pending until it ends
    ok("sbatch", &["--uid", "55636", "--gid", "3555", "--chdir", "/tmp", "--output", "/dev/null", "--wrap", "sleep 8"]);
    let out = "/tmp/slurm-req-held.out";
    fs::write(out, "").unwrap();
    ok("chmod", &["666", out]);
    let id = ok("sbatch", &["--parsable", "--chdir", "/tmp", "--output", out, "--uid", "600012372", "--gid", "15355", "--wrap", "echo ran as $(id -un)"]);
    let id = id.trim().to_string();

    let g = RequirePropDir::new();
    fs::remove_dir_all("/fake-exp/FXE/202201/p001237").unwrap();
    let mut reason = String::new();
    for _ in 0..60 {
        reason = job_field(&id, "Reason");
        if reason == "launch_failed_requeued_held" {
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    assert_eq!(reason, "launch_failed_requeued_held", "slurmd log:\n{}", fs::read_to_string("/var/log/slurm/slurmd.log").unwrap_or_default());
    assert_eq!(job_field(&id, "JobState"), "PENDING");
    assert_eq!(job_field(&id, "Restarts"), "1");
    assert_eq!(node_state(), "idle");

    // recovery: dir and config back, release the hold
    drop(g);
    fs::create_dir_all(format!("{EXP}/scratch")).unwrap();
    // slurmd remembers the failed launch of this job id for a while ("already killed, do not
    // launch batch job"): a release straight away fails the job with exit code 1; 25 s worked, 0 s did not
    std::thread::sleep(std::time::Duration::from_secs(30));
    ok("scontrol", &["release", &id]);
    // the launch-failure requeue also defers the job (Reason=BeginTime); start it now
    ok("scontrol", &["update", &format!("JobId={id}"), "StartTime=now"]);
    let mut state = String::new();
    for _ in 0..60 {
        state = job_field(&id, "JobState");
        if state == "COMPLETED" {
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    assert_eq!(state, "COMPLETED", "{}", job_field(&id, "Reason"));
    assert!(fs::read_to_string(out).unwrap().contains("ran as euxfel-p001237"));
    assert_eq!(node_state(), "idle");
}

/// Epilog: pkill -KILL -U $SLURM_UID and find -user $SLURM_UID rm -rf work with the synthetic uid.
#[test]
fn prolog_epilog_cleans_synthetic_uid_files_and_processes() {
    let cmd = "touch /tmp/epilog-junk /var/tmp/epilog-junk /dev/shm/epilog-junk; \
               setsid nohup sleep 4242 >/dev/null 2>&1 & sleep 1";
    clear(7);
    assert!(job("epilog", HAS_DIR, "debug", cmd).0);
    for f in ["/tmp/epilog-junk", "/var/tmp/epilog-junk", "/dev/shm/epilog-junk"] {
        assert!(wait_gone(f), "{f} survived; epilog log:\n{}", log("epilog", 7));
    }
    let mut alive = true;
    for _ in 0..30 {
        alive = run("sh", &["-c", "ps -u 600012372 -o stat= | grep -qv ^Z"]).0 /* zombies (unreaped by PID 1) do not count */;
        if !alive {
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    assert!(!alive, "orphan sleep survived; epilog log:\n{}", log("epilog", 7));
    assert_eq!(node_state(), "idle");
}
