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
             inner=$(sbatch --parsable --wait --chdir={d} --output={d}/inner.out --wrap 'echo \"inner uid=$(id -u) user=$(id -un)\"')\n\
             scontrol show job \"$inner\" | grep -o 'UserId=[^ ]*'\n"
        ),
    )
    .unwrap();
    ok("chmod", &["755", &format!("{d}/outer.sh")]);

    ok("sbatch", &["--uid", U, "--gid", "15355", "--wait", "--chdir", d, "--output", &format!("{d}/outer.out"), &format!("{d}/outer.sh")]);

    let outer = fs::read_to_string(format!("{d}/outer.out")).unwrap();
    let inner = fs::read_to_string(format!("{d}/inner.out")).unwrap();
    assert!(outer.contains(&format!("outer uid={U} user={N}")), "{outer}");
    assert!(outer.contains(&format!("UserId={N}({U})")), "{outer}");
    assert!(inner.contains(&format!("inner uid={U} user={N}")), "{inner}");
}
