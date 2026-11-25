// Needs sudo, the spawner/other users and sudoers/proposal-spawner; see Dockerfile.
mod common;
use common::{fails, ok};

const N: &str = "euxfel-p001234";

// runuser (as root) starts the session as `user`, then sudo does its own authorisation
fn sudo_as(user: &str, args: &[&str]) -> (&'static str, Vec<String>) {
    let mut v: Vec<String> = ["-u", user, "--", "sudo", "-n"].map(String::from).into();
    v.extend(args.iter().map(|a| a.to_string()));
    ("runuser", v)
}

fn sudo_ok(user: &str, args: &[&str]) -> String {
    let (c, v) = sudo_as(user, args);
    ok(c, &v.iter().map(String::as_str).collect::<Vec<_>>())
}

fn sudo_fails(user: &str, args: &[&str]) {
    let (c, v) = sudo_as(user, args);
    fails(c, &v.iter().map(String::as_str).collect::<Vec<_>>())
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
    let out = sudo_ok("xdamnprd", &["-u", N, "id"]);
    assert!(out.contains("uid=600012342(euxfel-p001234)"), "{out}");
    assert!(out.contains("gid=15355(exfel_prop_syn)"), "{out}");
}

#[test]
fn spawner_cannot_become_anything_else() {
    sudo_fails("xdamnprd", &["-u", "euxfel-p001235", "id"]); // no proposal group
    sudo_fails("xdamnprd", &["-u", "root", "id"]);
    sudo_fails("xdamnprd", &["-u", "nobody", "id"]);
    sudo_fails("xdamnprd", &["id"]); // plain sudo = root
    sudo_fails("xdamnprd", &["-u", N, "-g", "root", "id"]); // no group switch
}

#[test]
fn other_users_cannot_become_proposal_user() {
    // same primary group as the spawner, so the rule must be by user, not group
    sudo_fails("otheruser", &["-u", N, "id"]);
    sudo_fails(N, &["-u", N, "id"]);
}
