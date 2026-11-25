// Needs sudo, the spawner/other users and sudoers/proposal-spawner; see Dockerfile.
mod common;
use common::{ok, run};

const N: &str = "euxfel-p001234";

// runuser (as root) starts the session as `user`, then sudo does its own authorisation
fn sudo<'a>(user: &'a str, args: &[&'a str]) -> (bool, String) {
    let mut v = vec!["-u", user, "--", "sudo", "-n"];
    v.extend(args);
    run("runuser", &v)
}

#[test]
fn sudoers_is_valid() {
    ok("visudo", &["-c"]);
}

#[test]
fn spawner_has_expected_identity() {
    assert_eq!(ok("id", &["xdamnprd"]).trim(), "uid=55635(xdamnprd) gid=3555(exfel) groups=3555(exfel)");
}

#[test]
fn spawner_can_become_proposal_user() {
    let (success, out) = sudo("xdamnprd", &["-u", N, "id"]);
    assert!(success, "{out}");
    assert!(out.contains("uid=600012342(euxfel-p001234)"), "{out}");
    assert!(out.contains("gid=15355(exfel_prop_syn)"), "{out}");
}

#[test]
fn spawner_cannot_become_anything_else() {
    assert!(!sudo("xdamnprd", &["-u", "euxfel-p001235", "id"]).0); // no proposal group
    assert!(!sudo("xdamnprd", &["-u", "root", "id"]).0);
    assert!(!sudo("xdamnprd", &["-u", "nobody", "id"]).0);
    assert!(!sudo("xdamnprd", &["id"]).0); // plain sudo = root
    assert!(!sudo("xdamnprd", &["-u", N, "-g", "root", "id"]).0); // no group switch
}

#[test]
fn other_users_cannot_become_proposal_user() {
    // same primary group as the spawner, so the rule must be by user, not group
    assert!(!sudo("otheruser", &["-u", N, "id"]).0);
    assert!(!sudo(N, &["-u", N, "id"]).0);
}
