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

/// With LaunchParameters=disable_send_gids slurmd resolves groups itself (initgroups via NSS).
#[test]
fn proposal_user_job_gets_supplementary_groups() {
    let (accepted, out) = job("groups", PROP, "debug", "id -G; getent initgroups euxfel-p001234");
    assert!(accepted, "{out}");
    // primary exfel_prop_syn, supplementary exfel, plus the proposal group 60001234-part
    assert!(out.starts_with("15355 3555 600012342\n"), "{out}");
}
