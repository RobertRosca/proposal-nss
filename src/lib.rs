use libc::uid_t;
use libnss::group::Group;
use libnss::initgroups::InitgroupsHooks;
use libnss::interop::Response;
use libnss::passwd::{Passwd, PasswdHooks};
use libnss::{libnss_initgroups_hooks, libnss_passwd_hooks};

const CONF: &str = "/etc/proposal-nss.conf";
const EXP_DIR: &str = "/gpfs/exfel/exp";
const HOME_FALLBACK_DIR: &str = "/tmp";

struct Conf {
    primary_gid: u32,
    supp_gid: u32,
    exp_dir: String,
    require_prop_dir: bool,
    home_fallback_dir: String,
}

// `key = value` lines, `#` comments. GID keys required, `exp_dir`, `require_prop_dir` and `home_fallback_dir` optional, unknown keys rejected, so a
// typo fails closed (no users) instead of silently handing out wrong groups.
fn parse(text: &str) -> Option<Conf> {
    let (mut primary, mut supp, mut exp_dir, mut require) = (None, None, EXP_DIR.to_string(), true);
    let mut fallback = HOME_FALLBACK_DIR.to_string();
    for line in text.lines().map(|l| l.split('#').next().unwrap_or("").trim()).filter(|l| !l.is_empty()) {
        let (k, v) = line.split_once('=')?;
        let v = v.trim();
        match k.trim() {
            "primary_gid" => primary = Some(v.parse().ok()?),
            "supplementary_gid" => supp = Some(v.parse().ok()?),
            "exp_dir" if v.starts_with('/') => exp_dir = v.trim_end_matches('/').to_string(),
            "require_prop_dir" => require = v.parse().ok()?,
            "home_fallback_dir" if v.starts_with('/') => fallback = v.trim_end_matches('/').to_string(),
            _ => return None,
        }
    }
    Some(Conf {
        primary_gid: primary?,
        supp_gid: supp?,
        exp_dir,
        require_prop_dir: require,
        home_fallback_dir: fallback,
    })
}

// Why a lookup could not be answered; NotFound is reserved for "answered: absent", so Slurm holds
// a launch on a transient failure instead of treating the user as gone.
#[derive(Debug, PartialEq)]
enum Fail {
    TryAgain,
    Unavail,
}

// Read per lookup: the file is tiny and this keeps Puppet changes live without a restart. A missing
// or invalid file means no users (Ok(None)); any other read error is Unavail.
fn conf() -> Result<Option<Conf>, Fail> {
    match std::fs::read_to_string(conf_path()) {
        Ok(t) => Ok(parse(&t)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(Fail::Unavail),
    }
}

// Test builds (`--features test-conf`) let each test point the module at its own conf. Release
// builds have no environment lookup, so setuid callers (sudo) cannot redirect it.
fn conf_path() -> std::ffi::OsString {
    #[cfg(feature = "test-conf")]
    if let Some(p) = std::env::var_os("PROPOSAL_NSS_CONF") {
        return p;
    }
    CONF.into()
}

// uid == gid of the externally managed proposal group: `60{no:06}2`.
fn id_of(no: u32) -> u32 {
    600_000_002 + 10 * no
}

fn id_to_no(id: u32) -> Option<u32> {
    let d = id.checked_sub(600_000_002).filter(|d| d % 10 == 0)?;
    Some(d / 10).filter(|n| *n < 1_000_000)
}

// The proposal group is managed elsewhere (LDAP); a user exists only if it does.
// We expose no groups, so this lookup cannot recurse into this module.
// A getgrgid_r failure (sssd/LDAP EAGAIN/EIO, ERANGE at the cap) is TryAgain; POSIX also lets
// ENOENT/ESRCH/EBADF/EPERM mean "no such group".
fn exists(no: u32) -> Result<bool, Fail> {
    use nix::errno::Errno::{EBADF, ENOENT, EPERM, ESRCH};
    match nix::unistd::Group::from_gid(id_of(no).into()) {
        Ok(g) => Ok(g.is_some()),
        Err(ENOENT | ESRCH | EBADF | EPERM) => Ok(false),
        Err(_) => Err(Fail::TryAgain),
    }
}

// With `require_prop_dir` (default; hosts that mount the exp tree) the proposal directory must exist
// too, so nobody can switch to a proposal user without one. Hosts without the tree turn it off.
// Returns the proposal dirs found, scanned when `require_prop_dir` or `need_dirs` (the caller builds
// a passwd entry), so the exp tree is read once per lookup.
fn live(no: u32, c: &Conf, need_dirs: bool) -> Result<Option<Vec<std::path::PathBuf>>, Fail> {
    if !exists(no)? {
        return Ok(None);
    }
    if !(c.require_prop_dir || need_dirs) {
        return Ok(Some(vec![]));
    }
    match proposal_dirs(no, &c.exp_dir) {
        Ok(d) if c.require_prop_dir && d.is_empty() => Ok(None),
        Ok(d) => Ok(Some(d)),
        Err(e) if c.require_prop_dir => Err(e),
        Err(_) => Ok(Some(vec![])),
    }
}

fn name_to_no(name: &str) -> Option<u32> {
    let digits = name.strip_prefix("euxfel-p")?;
    if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

// Home = {exp_dir}/{instrument}/{cycle}/p{no:06}/scratch/.exfl_prop_syn-home. The uid only encodes
// the proposal number, so find instrument/cycle by scanning {exp_dir}/*/*/p{no:06} (one readdir
// per instrument; no caching, so new proposals appear immediately). Several matches (a proposal
// moved between cycles/instruments) resolve to the lexicographically smallest path so the answer
// is stable. No match, or any I/O error (GPFS unmounted, permissions): {home_fallback_dir}/{name}. With
// `require_prop_dir = false` the user is still returned (its existence is decided by the group, not
// the filesystem), and we never panic or block on anything but readdir/stat. With
// `require_prop_dir`, an I/O error other than ENOENT is Unavail rather than "no such proposal".
fn proposal_dirs(no: u32, exp_dir: &str) -> Result<Vec<std::path::PathBuf>, Fail> {
    use std::io::ErrorKind::{NotADirectory, NotFound};
    let absent = |e: &std::io::Error| matches!(e.kind(), NotFound | NotADirectory); // ENOTDIR: a file where a dir is expected
    // Errors do not abort the scan (one unreadable sibling must not hide a readable proposal); they
    // are remembered and only matter when nothing matched.
    let mut err = false;
    let p = format!("p{no:06}");
    let mut found = vec![];
    for c in walkdir::WalkDir::new(exp_dir).follow_links(true).min_depth(2).max_depth(2) {
        let d = match c {
            Ok(c) => c.into_path().join(&p),
            Err(e) => {
                err |= !e.io_error().is_some_and(absent); // a symlink loop has no io error
                continue;
            }
        };
        match std::fs::metadata(&d) {
            Ok(m) if m.is_dir() => found.push(d),
            Ok(_) => {}
            Err(e) => err |= !absent(&e),
        }
    }
    if found.is_empty() && err { Err(Fail::Unavail) } else { Ok(found) }
}

// Without a proposal dir each user gets its own {home_fallback_dir}/{name} (default /tmp), so
// mkhomedir_helper never creates one shared, wrongly owned home such as /nonexistent. A squatted
// fallback (see owned_home) becomes "/": root-owned, so mkhomedir_helper leaves it alone and nobody
// else controls the user's dotfiles; the user stays resolvable, with a HOME it cannot write.
fn home(dirs: Vec<std::path::PathBuf>, name: &str, uid: u32, c: &Conf) -> String {
    dirs.into_iter()
        .min()
        .and_then(|d| d.join("scratch/.exfl_prop_syn-home").to_str().map(String::from))
        .unwrap_or_else(|| {
            let f = format!("{}/{name}", c.home_fallback_dir);
            // `home_fallback_dir = /` (trimmed to "") turns per-user fallback homes off
            if !c.home_fallback_dir.is_empty() && owned_home(&f, uid) {
                f
            } else {
                "/".into()
            }
        })
}

// The fallback prefix may be world-writable (/tmp): anyone can pre-create {prefix}/{name}, and
// mkhomedir_helper keeps an existing path. Accept it only if absent (mkhomedir_helper creates it)
// or a real dir (lstat: no symlink) owned by `uid` that group and others cannot write (all proposal
// users share their primary group). Lookup-time only: the path can still appear before
// mkhomedir_helper runs (TOCTOU), so a root-owned home_fallback_dir is the real fix.
fn owned_home(p: &str, uid: u32) -> bool {
    use std::os::unix::fs::MetadataExt;
    match std::fs::symlink_metadata(p) {
        Ok(m) => m.is_dir() && m.uid() == uid && m.mode() & 0o022 == 0,
        Err(e) => e.kind() == std::io::ErrorKind::NotFound,
    }
}

fn passwd(no: u32, c: &Conf, dirs: Vec<std::path::PathBuf>) -> Passwd {
    let name = format!("euxfel-p{no:06}");
    Passwd {
        dir: home(dirs, &name, id_of(no), c),
        name,
        passwd: "*".into(), // locked; "x" makes pam_unix look up a shadow entry we don't provide
        uid: id_of(no),
        gid: c.primary_gid,
        gecos: format!("p{no:06} user"),
        shell: "/sbin/nologin".into(),
    }
}

// initgroups only reads the gid; the real entries live in other NSS sources.
fn member(gid: u32) -> Group {
    Group {
        name: String::new(),
        passwd: "x".into(),
        gid,
        members: vec![],
    }
}

// glibc hands the callers the errno left behind on TryAgain/Unavail, and libnss leaves errno
// untouched on non-success in passwd lookups (a stale one, e.g. EISDIR from our own read, was seen
// leaking), so set it here: EAGAIN for TryAgain (never ERANGE, which means "grow the buffer"), EIO
// for Unavail. initgroups overwrites it with ENOENT in the libnss glue (initgroups.rs:37,45).
fn found<T>(v: Result<Option<T>, Fail>) -> Response<T> {
    use nix::errno::Errno;
    match v {
        Ok(v) => v.map_or(Response::NotFound, Response::Success),
        Err(Fail::TryAgain) => {
            Errno::EAGAIN.set();
            Response::TryAgain
        }
        Err(Fail::Unavail) => {
            Errno::EIO.set();
            Response::Unavail
        }
    }
}

// Cheap id/name check first, so misses and real users never read the conf; then conf, then liveness.
fn lookup<T>(no: Option<u32>, need_dirs: bool, build: impl FnOnce(u32, &Conf, Vec<std::path::PathBuf>) -> T) -> Response<T> {
    found((|| {
        let Some(n) = no else { return Ok(None) };
        let Some(c) = conf()? else { return Ok(None) };
        Ok(live(n, &c, need_dirs)?.map(|d| build(n, &c, d)))
    })())
}

struct ProposalPasswd;
libnss_passwd_hooks!(proposal, ProposalPasswd);
impl PasswdHooks for ProposalPasswd {
    // no enumeration of 1M synthetic users
    fn get_all_entries() -> Response<Vec<Passwd>> {
        Response::Success(vec![])
    }
    fn get_entry_by_uid(uid: uid_t) -> Response<Passwd> {
        lookup(id_to_no(uid), true, passwd)
    }
    fn get_entry_by_name(name: String) -> Response<Passwd> {
        lookup(name_to_no(&name), true, passwd)
    }
}

struct ProposalInitgroups;
libnss_initgroups_hooks!(proposal, ProposalInitgroups);
impl InitgroupsHooks for ProposalInitgroups {
    fn get_entries_by_user(user: String) -> Response<Vec<Group>> {
        lookup(name_to_no(&user), false, |n, c, _| vec![member(id_of(n)), member(c.supp_gid)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapping() {
        assert_eq!(id_to_no(600_000_001), None);
        assert_eq!(id_to_no(600_000_002), Some(0));
        assert_eq!(id_to_no(609_005_112), Some(900_511));
        assert_eq!(id_to_no(609_005_111), None);
        assert_eq!(id_to_no(609_999_992), Some(999_999));
        assert_eq!(id_to_no(610_000_002), None);
        let c = parse("# c\nprimary_gid = 15355 # x\nsupplementary_gid=3555\n").unwrap();
        assert_eq!((c.primary_gid, c.supp_gid), (15355, 3555));
        for bad in [
            "",
            "primary_gid=1",
            "primary_gid=1\nsupplementary_gid=x",
            "primary_gid=1\nsupplementary_gid=2\nfoo=3",
            "primary_gid=1\nsupplementary_gid",
        ] {
            assert!(parse(bad).is_none(), "{bad:?}");
        }
        assert_eq!(c.exp_dir, EXP_DIR);
        assert_eq!(parse("primary_gid=1\nsupplementary_gid=2\nexp_dir = /x/y/ # c").unwrap().exp_dir, "/x/y");
        assert!(parse("primary_gid=1\nsupplementary_gid=2\nexp_dir=rel").is_none());
        assert!(c.require_prop_dir);
        assert!(
            parse("primary_gid=1\nsupplementary_gid=2\nrequire_prop_dir = true")
                .unwrap()
                .require_prop_dir
        );
        assert!(
            !parse("primary_gid=1\nsupplementary_gid=2\nrequire_prop_dir=false")
                .unwrap()
                .require_prop_dir
        );
        assert!(parse("primary_gid=1\nsupplementary_gid=2\nrequire_prop_dir=yes").is_none());
        let p = passwd(900_511, &c, vec![]);
        assert_eq!((p.name.as_str(), p.uid, p.gid), ("euxfel-p900511", 609_005_112, 15355));
        assert_eq!(p.gecos, "p900511 user");
        // no proposal dir: per-user home under home_fallback_dir (default /tmp, overridable, absolute only)
        assert_eq!(p.dir, "/tmp/euxfel-p900511");
        let o = parse("primary_gid=1\nsupplementary_gid=2\nhome_fallback_dir = /scratch/fb/ # c").unwrap();
        assert_eq!(passwd(900_511, &o, vec![]).dir, "/scratch/fb/euxfel-p900511");
        assert!(parse("primary_gid=1\nsupplementary_gid=2\nhome_fallback_dir=tmp").is_none());
        let off = parse("primary_gid=1\nsupplementary_gid=2\nhome_fallback_dir = /").unwrap();
        assert_eq!(passwd(900_511, &off, vec![]).dir, "/");
        let d = std::path::PathBuf::from("/x/FXE/202201/p900511");
        assert_eq!(passwd(900_511, &o, vec![d]).dir, "/x/FXE/202201/p900511/scratch/.exfl_prop_syn-home");
        assert_eq!(name_to_no(&p.name), Some(900_511));
        for bad in ["euxfel-p1234", "euxfel-p1000000", "euxfel-p+12345", "euxfel-p00123x", "root"] {
            assert_eq!(name_to_no(bad), None, "{bad}");
        }
    }

    #[test]
    fn failures_map_to_status() {
        assert_eq!(found(Ok(None::<u32>)), Response::NotFound);
        assert_eq!(found(Ok(Some(1))), Response::Success(1));
        use nix::errno::Errno;
        assert_eq!(found::<u32>(Err(Fail::Unavail)), Response::Unavail);
        assert_eq!(Errno::last(), Errno::EIO);
        assert_eq!(found::<u32>(Err(Fail::TryAgain)), Response::TryAgain);
        assert_eq!(Errno::last(), Errno::EAGAIN);
    }

    #[test]
    fn proposal_dirs_distinguishes_absent_from_error() {
        let tmp = tempfile::tempdir().unwrap();
        let t = tmp.path();
        std::fs::create_dir_all(t.join("FXE/202230/p001234")).unwrap();
        let root = t.to_str().unwrap();
        assert_eq!(proposal_dirs(1234, root).unwrap(), vec![t.join("FXE/202230/p001234")]);
        assert_eq!(proposal_dirs(1235, root).unwrap(), Vec::<std::path::PathBuf>::new());
        assert_eq!(
            proposal_dirs(1234, t.join("absent").to_str().unwrap()).unwrap(),
            Vec::<std::path::PathBuf>::new()
        );
    }

    #[test]
    fn fallback_home_refused_unless_owned() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let tmp = tempfile::tempdir().unwrap();
        let t = tmp.path();
        std::fs::create_dir(t.join("own")).unwrap();
        std::fs::set_permissions(t.join("own"), std::fs::Permissions::from_mode(0o755)).unwrap();
        symlink(t.join("own"), t.join("link")).unwrap();
        let me = nix::unistd::geteuid().as_raw();
        let p = |n: &str| t.join(n).to_str().unwrap().to_string();
        assert!(owned_home(&p("absent"), me));
        assert!(owned_home(&p("own"), me));
        assert!(!owned_home(&p("own"), me + 1), "other owner");
        assert!(!owned_home(&p("link"), me), "symlink to own dir");
        for mode in [0o775, 0o757, 0o777] {
            std::fs::set_permissions(t.join("own"), std::fs::Permissions::from_mode(mode)).unwrap();
            assert!(!owned_home(&p("own"), me), "{mode:o}");
        }
        std::fs::write(t.join("file"), "").unwrap();
        assert!(!owned_home(&p("file"), me), "plain file");
        assert!(!owned_home(&p("file/sub"), me), "ENOTDIR");
        // a dir the test made is never owned by the proposal uid, so passwd hands out "/"
        let c = parse(&format!("primary_gid=1\nsupplementary_gid=2\nhome_fallback_dir={}", t.display())).unwrap();
        std::fs::create_dir(t.join("euxfel-p900511")).unwrap();
        assert_eq!(passwd(900_511, &c, vec![]).dir, "/");
        assert_eq!(passwd(900_512, &c, vec![]).dir, p("euxfel-p900512"));
    }

    #[test]
    fn proposal_dirs_survives_unreadable_sibling() {
        // root bypasses mode 000 (container tier runs unit tests as root), so nothing to test there
        if nix::unistd::geteuid().is_root() {
            return;
        }
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let t = tmp.path();
        std::fs::create_dir_all(t.join("FXE/202230/p001234")).unwrap();
        std::fs::create_dir_all(t.join("AAA/202230")).unwrap();
        std::fs::set_permissions(t.join("AAA"), std::fs::Permissions::from_mode(0o000)).unwrap();
        let root = t.to_str().unwrap();
        assert_eq!(proposal_dirs(1234, root).unwrap(), vec![t.join("FXE/202230/p001234")]);
        // nothing matched and an error occurred: Unavail, not "absent"
        assert_eq!(proposal_dirs(1235, root), Err(Fail::Unavail));
        // TempDir cannot remove the contents of a mode-000 dir
        std::fs::set_permissions(t.join("AAA"), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}
