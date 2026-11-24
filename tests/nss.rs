// Needs the module installed in nsswitch plus the external group fixtures; see test/in-userns.sh and Dockerfile.
mod common;
use common::{fails, ok};

const U: &str = "600012342";
const N: &str = "euxfel-p001234";
const CONF: &str = "/etc/proposal-nss.conf";
const HOME: &str = ".exfl_prop_syn-home";

/// Points the module at a fake exp tree through a conf of the test's own: a copy of
/// /etc/proposal-nss.conf plus `exp_dir`, read by the test-conf module build via PROPOSAL_NSS_CONF.
/// /etc is never written, so the nss tests run in parallel and never disturb other test binaries.
struct FakeExp(std::path::PathBuf, std::path::PathBuf);

impl FakeExp {
    fn new(name: &str, dirs: &[&str]) -> Self {
        let root = std::env::temp_dir().join(format!("proposal-nss-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for d in dirs {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        let conf = root.with_extension("conf");
        let base = std::fs::read_to_string(CONF).unwrap();
        std::fs::write(&conf, format!("{base}\nexp_dir = {}\n", root.display())).unwrap();
        // SAFETY: nextest runs each test in its own process and nothing else reads the env concurrently.
        unsafe { std::env::set_var("PROPOSAL_NSS_CONF", &conf) };
        Self(root, conf)
    }
    fn require(&self) {
        let cur = std::fs::read_to_string(&self.1).unwrap();
        std::fs::write(&self.1, format!("{cur}require_prop_dir = true\n")).unwrap();
    }
    fn home(&self, rel: &str) -> String {
        format!("{}/{rel}/scratch/{HOME}", self.0.display())
    }
}

impl Drop for FakeExp {
    fn drop(&mut self) {
        // SAFETY: as in new().
        unsafe { std::env::remove_var("PROPOSAL_NSS_CONF") };
        let _ = std::fs::remove_file(&self.1);
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
fn missing_proposal_dir_falls_back_but_user_stays_visible_without_require_prop_dir() {
    // the fixture conf sets require_prop_dir = false
    let t = FakeExp::new("missing", &["FXE/202230/p001235"]);
    assert_eq!(dir_of(N), "/nonexistent");
    drop(t);
    // exp_dir absent (GPFS not mounted): same fallback
    assert_eq!(dir_of(U), "/nonexistent");
}

#[test]
fn require_prop_dir_hides_user_without_proposal_dir() {
    let t = FakeExp::new("req-missing", &["FXE/202230/p001235"]);
    t.require();
    for k in [U, N] {
        fails("getent", &["passwd", k]);
    }
    fails("id", &[N]);
}

#[test]
fn require_prop_dir_hides_user_when_exp_tree_is_empty() {
    let t = FakeExp::new("req-empty", &[]);
    t.require();
    fails("getent", &["passwd", N]);
}

#[test]
fn require_prop_dir_keeps_user_with_proposal_dir() {
    let t = FakeExp::new("req-present", &["FXE/202230/p001234"]);
    t.require();
    for k in [U, N] {
        assert_eq!(dir_of(k), t.home("FXE/202230/p001234"));
    }
    assert!(ok("id", &[N]).contains("3555(exfel)"));
}

#[test]
fn require_prop_dir_still_needs_proposal_group() {
    let t = FakeExp::new("req-nogroup", &["FXE/202230/p001235"]);
    t.require();
    fails("getent", &["passwd", "euxfel-p001235"]);
}

#[test]
fn unreadable_conf_is_unavailable_not_absent() {
    // a conf of the test's own, missing and then a directory: read fails with EISDIR, not ENOENT (works as root too)
    let p = std::env::temp_dir().join(format!("proposal-nss-badconf-{}.conf", std::process::id()));
    let _ = std::fs::remove_dir(&p);
    // SAFETY: nextest runs each test in its own process and nothing else reads the env concurrently.
    unsafe { std::env::set_var("PROPOSAL_NSS_CONF", &p) };
    // missing conf stays an authoritative miss: rc 0, no result
    assert_eq!(getpwnam_rc(N), (0, true));
    std::fs::create_dir(&p).unwrap();
    // Unavail surfaces as a non-zero rc (EIO), distinct from the miss above
    assert_eq!(getpwnam_rc(N), (libc::EIO, true));
    let _ = std::fs::remove_dir(&p);
}

fn getpwnam_rc(name: &str) -> (i32, bool) {
    let n = std::ffi::CString::new(name).unwrap();
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut res = std::ptr::null_mut();
    let mut buf = vec![0 as libc::c_char; 4096];
    let rc = unsafe { libc::getpwnam_r(n.as_ptr(), &mut pw, buf.as_mut_ptr(), buf.len(), &mut res) };
    (rc, res.is_null())
}
