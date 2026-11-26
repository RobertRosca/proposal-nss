// Needs the module installed in nsswitch plus the external group fixtures; see tests/scripts/in-userns.sh and Dockerfile.
mod common;
use common::{fails, ok};
use nix::errno::Errno;
use nix::unistd::User;
use rstest::rstest;

const U: &str = "600012342";
const N: &str = "euxfel-p001234";
const CONF: &str = "/etc/proposal-nss.conf";
const HOME: &str = ".exfl_prop_syn-home";

/// Points the module at a fake exp tree through a conf of the test's own: a copy of
/// /etc/proposal-nss.conf plus `exp_dir`, read by the test-conf module build via PROPOSAL_NSS_CONF.
/// /etc is never written, so the nss tests run in parallel and never disturb other test binaries.
struct FakeExp(tempfile::TempDir, #[allow(dead_code)] tempfile::NamedTempFile); // .1 keeps the conf alive

impl FakeExp {
    fn new(dirs: &[&str], require: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        for d in dirs {
            std::fs::create_dir_all(root.path().join(d)).unwrap();
        }
        let conf = tempfile::NamedTempFile::new().unwrap();
        let base = std::fs::read_to_string(CONF).unwrap();
        std::fs::write(&conf, format!("{base}\nexp_dir = {}\nrequire_prop_dir = {require}\n", root.path().display())).unwrap();
        // SAFETY: nextest runs each test in its own process and nothing else reads the env concurrently.
        unsafe { std::env::set_var("PROPOSAL_NSS_CONF", conf.path()) };
        Self(root, conf)
    }
    fn home(&self, rel: &str) -> String {
        format!("{}/{rel}/scratch/{HOME}", self.0.path().display())
    }
}

impl Drop for FakeExp {
    fn drop(&mut self) {
        // SAFETY: as in new().
        unsafe { std::env::remove_var("PROPOSAL_NSS_CONF") };
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
fn no_proposal_group_means_no_user() {
    // 1235 has no external group; the rest are malformed or out of range
    for k in ["600012352", "euxfel-p001235", "599999999", "610000002", "600012341", "euxfel-p1234"] {
        fails("getent", &["passwd", k]);
    }
    fails("id", &["euxfel-p001235"]);
}

enum Want {
    Home(&'static str),
    Nonexistent,
    Absent,
}

// The fixture conf sets require_prop_dir = false; `require` overrides it. Proposal 1235 has no external group.
#[rstest]
// other proposals' dirs and plain files must not confuse the glob
#[case::home_from_instrument_and_cycle(&["FXE/202230/p001234", "FXE/202230/p001235", "SPB/202230/p009999"], false, 1234, Want::Home("FXE/202230/p001234"))]
#[case::multiple_matches_pick_smallest_path(&["SPB/202201/p001234", "FXE/202230/p001234", "FXE/202101/p001234"], false, 1234, Want::Home("FXE/202101/p001234"))]
#[case::missing_dir_falls_back_without_require(&["FXE/202230/p001235"], false, 1234, Want::Nonexistent)]
#[case::require_hides_user_without_dir(&["FXE/202230/p001235"], true, 1234, Want::Absent)]
#[case::require_keeps_user_with_dir(&["FXE/202230/p001234"], true, 1234, Want::Home("FXE/202230/p001234"))]
#[case::require_still_needs_group(&["FXE/202230/p001235"], true, 1235, Want::Absent)]
fn home_dir(#[case] dirs: &[&str], #[case] require: bool, #[case] no: u32, #[case] want: Want) {
    let t = FakeExp::new(dirs, require);
    std::fs::create_dir_all(t.0.path().join("FXE")).unwrap();
    std::fs::write(t.0.path().join("FXE/stray"), "").unwrap();
    let name = format!("euxfel-p{no:06}");
    for k in [format!("60{no:06}2"), name.clone()] {
        match &want {
            Want::Home(rel) => assert_eq!(dir_of(&k), t.home(rel)),
            Want::Nonexistent => assert_eq!(dir_of(&k), "/nonexistent"),
            Want::Absent => fails("getent", &["passwd", &k]),
        }
    }
    match want {
        Want::Absent => fails("id", &[&name]),
        _ => assert!(ok("id", &[&name]).contains("3555(exfel)")),
    }
}

#[test]
fn missing_exp_dir_falls_back() {
    // exp_dir absent (GPFS not mounted): same fallback as a missing proposal dir
    let t = FakeExp::new(&[], false);
    std::fs::remove_dir(t.0.path()).unwrap();
    assert_eq!(dir_of(U), "/nonexistent");
}

/// Points the module at a conf of the test's own that is missing, or a directory (read fails with
/// EISDIR, not ENOENT, even as root) for `unreadable()`. /etc is never touched.
struct BadConf {
    _tmp: tempfile::TempDir,
}

impl BadConf {
    fn at(dir: bool) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("conf");
        if dir {
            std::fs::create_dir(&p).unwrap();
        }
        // SAFETY: nextest runs each test in its own process and nothing else reads the env concurrently.
        unsafe { std::env::set_var("PROPOSAL_NSS_CONF", &p) };
        Self { _tmp: tmp }
    }
    fn missing() -> Self {
        Self::at(false)
    }
    fn unreadable() -> Self {
        Self::at(true)
    }
}

impl Drop for BadConf {
    fn drop(&mut self) {
        // SAFETY: as in at().
        unsafe { std::env::remove_var("PROPOSAL_NSS_CONF") };
    }
}

#[test]
fn missing_conf_is_a_miss() {
    let _c = BadConf::missing();
    assert_eq!(User::from_name(N), Ok(None));
}

#[test]
fn unreadable_conf_is_unavailable_not_absent() {
    let _c = BadConf::unreadable();
    assert_eq!(User::from_name(N), Err(Errno::EIO));
}

#[test]
fn non_proposal_names_do_not_read_conf() {
    let _c = BadConf::unreadable();
    for k in ["nosuchuser", "euxfel-p1234", "euxfel-p00123x"] {
        assert_eq!(User::from_name(k), Ok(None), "{k}");
    }
}
