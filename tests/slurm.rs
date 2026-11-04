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
#[ignore = "known issue: AllowGroups=exfel_prop_syn denies proposal users (no passwd enumeration)"]
fn desired_allowgroups_primary_group_accepts_proposal_user() {
    let (accepted, err) = job("want-syn", PROP, "syn", "true");
    assert!(accepted, "{err}");
}

#[test]
#[ignore = "known issue: AllowGroups=exfel denies proposal users (supplementary group via initgroups only)"]
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
