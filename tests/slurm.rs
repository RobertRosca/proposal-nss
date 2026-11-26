// Needs a running single-node Slurm; see Dockerfile and .config/nextest.toml.
mod common;
use common::slurm::{assert_lookup_identical, clear, exp_fixture, job, log, node_state, prop, wait, EXP, SCRATCH};
use common::{ok, run};
use rstest::rstest;
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
    wait(30, || {
        inner = fs::read_to_string(format!("{d}/inner.out")).unwrap();
        !inner.is_empty()
    });
    assert!(outer.contains(&format!("outer uid={U} user={N}")), "{outer}");
    assert!(outer.contains(&format!("UserId={N}({U})")), "{outer}");
    assert!(inner.contains(&format!("inner uid={U} user={N}")), "{inner}");
}

const OTHER: [&str; 4] = ["--uid", "55636", "--gid", "3555"]; // real user, primary group exfel (3555)

const DENIED: &str = "User's group not permitted to use this partition";

// Partition AllowGroups: slurmctld builds the allowed uid set from each group's gr_mem plus a
// getpwent scan for users whose primary gid matches. The module has no enumeration and no
// group entries with members, so proposal users are denied everywhere; observed:
//   sbatch: error: Batch job submission failed: User's group not permitted to use this partition
// Resolution: append exfel_prop_syn to a populated AllowGroups, see
// allowgroups_real_plus_synthetic_group_accepts_proposal_user and docs/deployment.md. The
// single-group cases of partition_access stay denied by design (a supplementary group is only
// set via initgroups, which slurmctld never consults).
// GroupUpdateTime=1 in setup-slurm.sh keeps slurmctld's group cache from hiding changes.

/// `who` is a sbatch --uid/--gid pair; `expected` is an error substring (Some("") = any rejection), None = accepted.
/// otheruser (55636) has primary gid exfel, found by slurmctld's primary-gid scan of passwd.
/// outsider (55637) only has an association with account `other`; proposal 1244 likewise.
#[rstest]
#[case::syn_denies_proposal_user("syn", prop(1234), Some(DENIED))]
#[case::sup_denies_proposal_user("sup", prop(1234), Some(DENIED))]
#[case::syn_denies_real_user("syn", OTHER.map(String::from).to_vec(), Some(DENIED))]
#[case::sup_accepts_real_user("sup", OTHER.map(String::from).to_vec(), None)]
#[case::acct_accepts_proposal_user("acct", prop(1234), None)]
#[case::acct_accepts_real_user("acct", OTHER.map(String::from).to_vec(), None)]
#[case::acct_rejects_outsider("acct", vec!["--uid".into(), "55637".into(), "--gid".into(), "3555".into()], Some(""))]
#[case::acct_rejects_proposal_user_of_other_account("acct", prop(1244), Some(""))]
fn partition_access(#[case] partition: &str, #[case] who: Vec<String>, #[case] expected: Option<&str>) {
    let (accepted, err) = job(&format!("pa-{partition}-{}", who[1]), &who, partition, "true");
    match expected {
        None => assert!(accepted, "{err}"),
        Some(e) => assert!(!accepted && err.contains(e), "{err}"),
    }
}

/// With LaunchParameters=disable_send_gids slurmd resolves groups itself (initgroups via NSS).
#[test]
fn proposal_user_job_gets_supplementary_groups() {
    let (accepted, out) = job("groups", &prop(1234), "debug", "id -G; getent initgroups euxfel-p001234");
    assert!(accepted, "{out}");
    // primary exfel_prop_syn, supplementary exfel, plus the proposal group 60001234-part
    assert!(out.starts_with("15355 3555 600012342\n"), "{out}");
}

// Accounting: the container runs slurmdbd with AccountingStorageEnforce=
// associations,limits like production. setup-slurm.sh gives otheruser, xdamnprd and euxfel-p001234
// an association in account `exfel`; `outsider` only has account `other`.

fn sacctmgr_add_user(no: u32, account: &str) {
    // sacctmgr resolves the synthetic name through the NSS module: this must work in the container
    ok("sacctmgr", &["-i", "add", "user", &format!("name=euxfel-p{no:06}"), &format!("account={account}")]);
}

const NO_ASSOC: &str = "Invalid account or account/partition combination specified";

#[test]
fn accounting_association_makes_proposal_user_accepted_at_once() {
    // observed: sbatch: error: Batch job submission failed: Invalid account or account/partition combination specified
    let (accepted, err) = job("pre-assoc", &prop(1241), "debug", "true");
    assert!(!accepted && err.contains(NO_ASSOC), "{err}");
    sacctmgr_add_user(1241, "exfel");
    // observed: slurmdbd pushes the new association to slurmctld within ~15 ms, no retry needed
    let (accepted, err) = job("post-assoc", &prop(1241), "debug", "true");
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
            .args(prop(no))
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
    wait(90, || {
        s = (state(&a1), state(&a2), state(&b1));
        s.0.starts_with("RUNNING") && s.2.starts_with("RUNNING") && s.1.contains("QOSMaxJobsPerUserLimit")
    });
    ok("scancel", &[&a1, &a2, &b1]);
    assert!(s.0.starts_with("RUNNING"), "first job of 1242: {s:?}");
    assert!(s.1.starts_with("PENDING") && s.1.contains("QOSMaxJobsPerUserLimit"), "second job of 1242: {s:?}");
    assert!(s.2.starts_with("RUNNING"), "first job of 1243: {s:?}");
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
    let (accepted, err) = job("prop-both", &prop(1234), "both", "true");
    assert!(accepted, "{err}");
    assert!(job("other-both", &OTHER, "both", "true").0);
}

// Production prolog/epilog (sanitised copies in tests/fixtures/prod-prolog, installed as /etc/slurm/slurm_{prolog,epilog}.sh,
// PrologFlags=Alloc,Contain,Serial). They trace to /var/log/slurm/{prolog,epilog}-test.<user>.log.
// Exp tree (outside /tmp: the epilog rm -rf deletes the user's files there): /fake-exp (exp_dir in /etc/proposal-nss.conf); p001237 exists there, p001238 does not.

/// The prolog checks /home/$SLURM_JOB_USER (never exists for synthetic users), so mkhomedir_helper
/// runs on every job; with the proposal dir present it creates the module's pw_dir.
#[test]
fn prolog_mkhomedir_creates_pw_dir_under_proposal_scratch() {
    exp_fixture();
    let home = format!("{EXP}/scratch/.exfl_prop_syn-home");
    let _ = fs::remove_dir_all(&home);
    clear(7);
    assert!(job("prolog-home", &prop(1237), "debug", "true").0);
    assert_eq!(node_state(), "idle");
    assert!(std::path::Path::new(&home).is_dir(), "{home} missing; prolog log:\n{}", log("prolog", 7));
    assert!(!std::path::Path::new("/home/euxfel-p001237").exists());
    // created through the scratch symlink, owned by the proposal user
    assert!(fs::symlink_metadata(format!("{EXP}/scratch")).unwrap().is_symlink());
    let real = format!("{SCRATCH}/.exfl_prop_syn-home");
    assert_eq!(std::os::unix::fs::MetadataExt::uid(&fs::metadata(&real).unwrap()), 600012372, "{real}");
}

#[test]
fn passwd_lookup_identical_for_root_slurm_and_non_member() {
    assert_lookup_identical();
}

/// Without a proposal dir pw_dir is the per-user fallback /tmp/euxfel-p001238: mkhomedir_helper
/// creates it for that user only (no shared /nonexistent), and the epilog's /tmp cleanup removes it
/// once the user's last job on the node ends.
#[test]
fn prolog_mkhomedir_creates_per_user_fallback_home_without_proposal_dir() {
    let home = "/tmp/euxfel-p001238";
    let _ = fs::remove_dir_all("/nonexistent");
    let _ = fs::remove_dir_all(home);
    clear(8);
    let cmd = "h=$(getent passwd $(id -u) | cut -d: -f6); echo home=$h; test -d $h -a -w $h && echo writable";
    let (accepted, out) = job("prolog-fallback", &prop(1238), "debug", cmd);
    assert!(accepted && out.contains(&format!("home={home}\nwritable")), "{out}\nprolog log:\n{}", log("prolog", 8));
    assert_eq!(node_state(), "idle", "{}", log("prolog", 8));
    assert!(!std::path::Path::new("/nonexistent").exists());
    assert!(!std::path::Path::new(home).exists(), "epilog left {home}:\n{}", log("epilog", 8));
}

/// Another user pre-creates the fallback home in world-writable /tmp: the module refuses it and
/// hands out "/", so mkhomedir_helper creates nothing and the squatter's dir stays untouched.
#[test]
fn prolog_refuses_fallback_home_squatted_by_other_user() {
    struct Squat(&'static str);
    impl Drop for Squat {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(self.0); // owned by otheruser, so the epilog leaves it
        }
    }
    let s = Squat("/tmp/euxfel-p001238");
    let _ = fs::remove_dir_all(s.0);
    fs::create_dir(s.0).unwrap();
    ok("chown", &["otheruser", s.0]);
    ok("chmod", &["755", s.0]);
    assert!(ok("getent", &["passwd", "euxfel-p001238"]).contains(":/:"));
    clear(8);
    let (accepted, out) = job("prolog-squat", &prop(1238), "debug", "echo home=$(getent passwd $(id -u) | cut -d: -f6)");
    assert!(accepted && out.contains("home=/\n"), "{out}\nprolog log:\n{}", log("prolog", 8));
    assert_eq!(node_state(), "idle", "{}", log("prolog", 8));
    assert_eq!(fs::read_dir(s.0).unwrap().count(), 0, "{} gained files", s.0);
}

/// Epilog: pkill -KILL -U $SLURM_UID and find -user $SLURM_UID rm -rf work with the synthetic uid.
#[test]
fn prolog_epilog_cleans_synthetic_uid_files_and_processes() {
    let cmd = "touch /tmp/epilog-junk /var/tmp/epilog-junk /dev/shm/epilog-junk; \
               setsid nohup sleep 4242 >/dev/null 2>&1 & sleep 1";
    clear(7);
    assert!(job("epilog", &prop(1237), "debug", cmd).0);
    for f in ["/tmp/epilog-junk", "/var/tmp/epilog-junk", "/dev/shm/epilog-junk"] {
        assert!(wait(30, || !std::path::Path::new(f).exists()), "{f} survived; epilog log:\n{}", log("epilog", 7));
    }
    // zombies (unreaped by PID 1) do not count
    let gone = wait(30, || !run("sh", &["-c", "ps -u 600012372 -o stat= | grep -qv ^Z"]).0);
    assert!(gone, "orphan sleep survived; epilog log:\n{}", log("epilog", 7));
    assert_eq!(node_state(), "idle");
}
