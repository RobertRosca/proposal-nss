// Needs the module installed in nsswitch plus the external group fixtures; see test/entrypoint.sh.
mod common;
use common::{fails, ok};

const U: &str = "600012342";
const N: &str = "euxfel-p001234";

#[test]
fn passwd_entry_by_uid_and_name() {
    let want = format!("{N}:*:{U}:15355:p001234 user:/nonexistent:/sbin/nologin");
    assert_eq!(ok("getent", &["passwd", U]).trim(), want);
    assert_eq!(ok("getent", &["passwd", N]).trim(), want);
}

#[test]
fn id_has_primary_proposal_and_supplementary_groups() {
    let out = ok("id", &[N]);
    for part in [&format!("uid={U}({N})")[..], "gid=15355(exfel_prop_syn)", "600012342(60001234-part)", "3555(exfel)"] {
        assert!(out.contains(part), "{part} missing from {out}");
    }
}

#[test]
fn module_exposes_no_groups() {
    fails("getent", &["group", "600012352"]);
    fails("getent", &["group", "60001235-part"]);
}

#[test]
fn no_proposal_group_means_no_user() {
    // 1235 has no external group; the rest are malformed or out of range
    for k in ["600012352", "euxfel-p001235", "599999999", "610000002", "600012341", "euxfel-p1234"] {
        fails("getent", &["passwd", k]);
    }
    fails("id", &["euxfel-p001235"]);
}
