// Needs the module installed in nsswitch plus the external group fixtures; see test/entrypoint.sh.
mod common;
use common::{fails, ok};

const U: &str = "600012342";
const N: &str = "euxfel-p001234";
const CONF: &str = "/etc/proposal-nss.conf";
const HOME: &str = ".exfl_prop_syn-home";

/// Points the module at a fake exp tree for the test's lifetime, then restores the conf.
/// Rewrites /etc/proposal-nss.conf: harmless in the per-test userns, so nextest serialises
/// the nss binary (test group) for the shared-/etc ci container.
struct FakeExp(std::path::PathBuf, String);

impl FakeExp {
    fn new(name: &str, dirs: &[&str]) -> Self {
        let root = std::env::temp_dir().join(format!("proposal-nss-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for d in dirs {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        let old = std::fs::read_to_string(CONF).unwrap();
        std::fs::write(CONF, format!("{old}
exp_dir = {}
", root.display())).unwrap();
        Self(root, old)
    }
    fn home(&self, rel: &str) -> String {
        format!("{}/{rel}/scratch/{HOME}", self.0.display())
    }
}

impl Drop for FakeExp {
    fn drop(&mut self) {
        let _ = std::fs::write(CONF, &self.1);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn dir_of(key: &str) -> String {
    ok("getent", &["passwd", key]).trim().split(':').nth(5).unwrap().to_string()
}

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

#[test]
fn home_resolved_from_instrument_and_cycle() {
    // other proposals' dirs and plain files must not confuse the glob
    let t = FakeExp::new("one", &["FXE/202230/p001234", "FXE/202230/p001235", "SPB/202230/p009999"]);
    std::fs::write(t.0.join("FXE/stray"), "").unwrap();
    for k in [U, N] {
        assert_eq!(dir_of(k), t.home("FXE/202230/p001234"));
    }
}

#[test]
fn multiple_matches_pick_smallest_path() {
    let t = FakeExp::new("multi", &["SPB/202201/p001234", "FXE/202230/p001234", "FXE/202101/p001234"]);
    assert_eq!(dir_of(N), t.home("FXE/202101/p001234"));
}

#[test]
fn missing_proposal_dir_falls_back_but_user_stays_visible() {
    let t = FakeExp::new("missing", &["FXE/202230/p001235"]);
    assert_eq!(dir_of(N), "/nonexistent");
    drop(t);
    // exp_dir absent (GPFS not mounted): same fallback
    assert_eq!(dir_of(U), "/nonexistent");
}
