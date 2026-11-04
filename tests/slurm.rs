// Needs a running single-node Slurm; see test/entrypoint.sh.
mod common;
use common::ok;
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
// KNOWN ISSUE (proposal-nss-dq4.3): desired behaviour is acceptance, see the ignored tests.
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
#[ignore = "still a known issue (dq4.3; AllowAccounts is the workaround): AllowGroups=exfel_prop_syn denies proposal users (no passwd enumeration)"]
fn desired_allowgroups_primary_group_accepts_proposal_user() {
    let (accepted, err) = job("want-syn", PROP, "syn", "true");
    assert!(accepted, "{err}");
}

#[test]
#[ignore = "still a known issue (dq4.3; AllowAccounts is the workaround): AllowGroups=exfel denies proposal users (supplementary group via initgroups only)"]
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

// Accounting (proposal-nss-dq4.1): the container runs slurmdbd with AccountingStorageEnforce=
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

// dq4.3 follow-up: AllowAccounts needs no group lookups, only the association.

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
/// Reading: Slurm only falls back to matching the user's primary gid against the listed groups when
/// the merged member-uid list is non-empty; exfel_prop_syn has no members, so alone the list is empty
/// and everything is denied. A real group with members next to it makes the gid match work. Works
/// without module enumeration, but relies on unspecified slurmctld behaviour: prefer AllowAccounts.
#[test]
fn allowgroups_real_plus_synthetic_group_accepts_proposal_user() {
    let (accepted, err) = job("prop-both", &who(&prop(1234)), "both", "true");
    assert!(accepted, "{err}");
    assert!(job("other-both", OTHER, "both", "true").0);
}
