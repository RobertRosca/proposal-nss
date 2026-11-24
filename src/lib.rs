use libc::uid_t;
use libnss::group::Group;
use libnss::initgroups::InitgroupsHooks;
use libnss::interop::Response;
use libnss::passwd::{Passwd, PasswdHooks};
use libnss::{libnss_initgroups_hooks, libnss_passwd_hooks};

const CONF: &str = "/etc/proposal-nss.conf";
const EXP_DIR: &str = "/gpfs/exfel/exp";

struct Conf {
    primary_gid: u32,
    supp_gid: u32,
    exp_dir: String,
    require_prop_dir: bool,
}

// `key = value` lines, `#` comments. GID keys required, `exp_dir` and `require_prop_dir` optional, unknown keys rejected, so a
// typo fails closed (no users) instead of silently handing out wrong groups.
fn parse(text: &str) -> Option<Conf> {
    let (mut primary, mut supp, mut exp_dir, mut require) = (None, None, EXP_DIR.to_string(), true);
    for line in text.lines().map(|l| l.split('#').next().unwrap_or("").trim()).filter(|l| !l.is_empty()) {
        let (k, v) = line.split_once('=')?;
        let v = v.trim();
        match k.trim() {
            "primary_gid" => primary = Some(v.parse().ok()?),
            "supplementary_gid" => supp = Some(v.parse().ok()?),
            "exp_dir" if v.starts_with('/') => exp_dir = v.trim_end_matches('/').to_string(),
            "require_prop_dir" => require = v.parse().ok()?,
            _ => return None,
        }
    }
    Some(Conf { primary_gid: primary?, supp_gid: supp?, exp_dir, require_prop_dir: require })
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
    let mut grp: libc::group = unsafe { std::mem::zeroed() };
    let mut res: *mut libc::group = std::ptr::null_mut();
    let mut buf = vec![0u8; 1024];
    loop {
        let rc = unsafe {
            libc::getgrgid_r(id_of(no), &mut grp, buf.as_mut_ptr().cast(), buf.len(), &mut res)
        };
        if rc == libc::ERANGE && buf.len() < 1 << 20 {
            buf.resize(buf.len() * 2, 0);
            continue;
        }
        return match rc {
            0 => Ok(!res.is_null()),
            libc::ENOENT | libc::ESRCH | libc::EBADF | libc::EPERM => Ok(false),
            _ => Err(Fail::TryAgain),
        };
    }
}

// With `require_prop_dir` (default; hosts that mount the exp tree) the proposal directory must exist
// too, so nobody can switch to a proposal user without one. Hosts without the tree turn it off.
fn live(no: Option<u32>, c: &Conf) -> Result<Option<u32>, Fail> {
    let Some(n) = no else { return Ok(None) };
    if !exists(n)? {
        return Ok(None);
    }
    if c.require_prop_dir && proposal_dirs(n, &c.exp_dir)?.is_empty() {
        return Ok(None);
    }
    Ok(Some(n))
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
// is stable. No match, or any I/O error (GPFS unmounted, permissions): "/nonexistent". With
// `require_prop_dir = false` the user is still returned (its existence is decided by the group, not
// the filesystem), and we never panic or block on anything but readdir/stat. With
// `require_prop_dir`, an I/O error other than ENOENT is Unavail rather than "no such proposal".
fn proposal_dirs(no: u32, exp_dir: &str) -> Result<Vec<std::path::PathBuf>, Fail> {
    use std::io::ErrorKind::{NotADirectory, NotFound};
    let absent = |e: &std::io::Error| matches!(e.kind(), NotFound | NotADirectory);
    // Errors do not abort the scan (one unreadable sibling must not hide a readable proposal); they
    // are remembered and only matter when nothing matched.
    let mut err = false;
    let mut subdirs = |d: &std::path::Path| {
        let mut out = vec![];
        match std::fs::read_dir(d) {
            Ok(rd) => {
                for e in rd {
                    match e {
                        Ok(e) => match std::fs::metadata(e.path()) {
                            Ok(m) if m.is_dir() => out.push(e.path()),
                            Ok(_) => {}
                            Err(e) => err |= !absent(&e),
                        },
                        Err(_) => err = true,
                    }
                }
            }
            Err(e) => err |= e.kind() != NotFound,
        }
        out
    };
    let p = format!("p{no:06}");
    let mut cycles = vec![];
    for i in subdirs(exp_dir.as_ref()) {
        cycles.extend(subdirs(&i));
    }
    let mut found = vec![];
    for c in cycles {
        let d = c.join(&p);
        match std::fs::metadata(&d) {
            Ok(m) if m.is_dir() => found.push(d),
            Ok(_) => {}
            Err(e) => err |= !absent(&e),
        }
    }
    if found.is_empty() && err { Err(Fail::Unavail) } else { Ok(found) }
}

fn home(no: u32, exp_dir: &str) -> String {
    proposal_dirs(no, exp_dir)
        .unwrap_or_default()
        .into_iter()
        .min()
        .and_then(|d| d.join("scratch/.exfl_prop_syn-home").to_str().map(String::from))
        .unwrap_or_else(|| "/nonexistent".into())
}

fn passwd(no: u32, c: &Conf) -> Passwd {
    Passwd {
        name: format!("euxfel-p{no:06}"),
        passwd: "*".into(), // locked; "x" makes pam_unix look up a shadow entry we don't provide
        uid: id_of(no),
        gid: c.primary_gid,
        gecos: format!("p{no:06} user"),
        dir: home(no, &c.exp_dir),
        shell: "/sbin/nologin".into(),
    }
}

// initgroups only reads the gid; the real entries live in other NSS sources.
fn member(gid: u32) -> Group {
    Group { name: String::new(), passwd: "x".into(), gid, members: vec![] }
}

// glibc hands the callers the errno left behind on TryAgain/Unavail, and libnss leaves errno
// untouched on non-success in passwd lookups (a stale one, e.g. EISDIR from our own read, was seen
// leaking), so set it here: EAGAIN for TryAgain (never ERANGE, which means "grow the buffer"), EIO
// for Unavail. initgroups overwrites it with ENOENT in the libnss glue (initgroups.rs:37,45).
fn found<T>(v: Result<Option<T>, Fail>) -> Response<T> {
    let set = |e| unsafe { *libc::__errno_location() = e };
    match v {
        Ok(v) => v.map_or(Response::NotFound, Response::Success),
        Err(Fail::TryAgain) => {
            set(libc::EAGAIN);
            Response::TryAgain
        }
        Err(Fail::Unavail) => {
            set(libc::EIO);
            Response::Unavail
        }
    }
}

// conf, then a live proposal number, then the entry built from both.
fn lookup<T>(no: impl FnOnce() -> Option<u32>, build: impl FnOnce(u32, &Conf) -> T) -> Response<T> {
    found((|| {
        let Some(c) = conf()? else { return Ok(None) };
        Ok(live(no(), &c)?.map(|n| build(n, &c)))
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
        lookup(|| id_to_no(uid), passwd)
    }
    fn get_entry_by_name(name: String) -> Response<Passwd> {
        lookup(|| name_to_no(&name), passwd)
    }
}

struct ProposalInitgroups;
libnss_initgroups_hooks!(proposal, ProposalInitgroups);
impl InitgroupsHooks for ProposalInitgroups {
    fn get_entries_by_user(user: String) -> Response<Vec<Group>> {
        lookup(|| name_to_no(&user), |n, c| vec![member(id_of(n)), member(c.supp_gid)])
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
        for bad in ["", "primary_gid=1", "primary_gid=1\nsupplementary_gid=x", "primary_gid=1\nsupplementary_gid=2\nfoo=3", "primary_gid=1\nsupplementary_gid"] {
            assert!(parse(bad).is_none(), "{bad:?}");
        }
        assert_eq!(c.exp_dir, EXP_DIR);
        assert_eq!(parse("primary_gid=1\nsupplementary_gid=2\nexp_dir = /x/y/ # c").unwrap().exp_dir, "/x/y");
        assert!(parse("primary_gid=1\nsupplementary_gid=2\nexp_dir=rel").is_none());
        assert!(c.require_prop_dir);
        assert!(parse("primary_gid=1\nsupplementary_gid=2\nrequire_prop_dir = true").unwrap().require_prop_dir);
        assert!(!parse("primary_gid=1\nsupplementary_gid=2\nrequire_prop_dir=false").unwrap().require_prop_dir);
        assert!(parse("primary_gid=1\nsupplementary_gid=2\nrequire_prop_dir=yes").is_none());
        let p = passwd(900_511, &c);
        assert_eq!((p.name.as_str(), p.uid, p.gid), ("euxfel-p900511", 609_005_112, 15355));
        assert_eq!(p.gecos, "p900511 user");
        assert_eq!(name_to_no(&p.name), Some(900_511));
        for bad in ["euxfel-p1234", "euxfel-p1000000", "euxfel-p+12345", "euxfel-p00123x", "root"] {
            assert_eq!(name_to_no(bad), None, "{bad}");
        }
    }

    #[test]
    fn failures_map_to_status() {
        assert_eq!(found(Ok(None::<u32>)), Response::NotFound);
        assert_eq!(found(Ok(Some(1))), Response::Success(1));
        let errno = || unsafe { *libc::__errno_location() };
        assert_eq!(found::<u32>(Err(Fail::Unavail)), Response::Unavail);
        assert_eq!(errno(), libc::EIO);
        assert_eq!(found::<u32>(Err(Fail::TryAgain)), Response::TryAgain);
        assert_eq!(errno(), libc::EAGAIN);
    }

    #[test]
    fn proposal_dirs_distinguishes_absent_from_error() {
        let t = std::env::temp_dir().join(format!("proposal-nss-unit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        std::fs::create_dir_all(t.join("FXE/202230/p001234")).unwrap();
        let root = t.to_str().unwrap();
        assert_eq!(proposal_dirs(1234, root).unwrap(), vec![t.join("FXE/202230/p001234")]);
        assert_eq!(proposal_dirs(1235, root).unwrap(), Vec::<std::path::PathBuf>::new());
        assert_eq!(proposal_dirs(1234, t.join("absent").to_str().unwrap()).unwrap(), Vec::<std::path::PathBuf>::new());
        // a file where the exp dir should be is an I/O error, not "no proposal"
        std::fs::write(t.join("file"), "").unwrap();
        assert_eq!(proposal_dirs(1234, t.join("file").to_str().unwrap()), Err(Fail::Unavail));
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn proposal_dirs_survives_unreadable_sibling() {
        // root bypasses mode 000 (container tier runs unit tests as root), so nothing to test there
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        use std::os::unix::fs::PermissionsExt;
        let t = std::env::temp_dir().join(format!("proposal-nss-unit-sib-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        std::fs::create_dir_all(t.join("FXE/202230/p001234")).unwrap();
        std::fs::create_dir_all(t.join("AAA/202230")).unwrap();
        std::fs::set_permissions(t.join("AAA"), std::fs::Permissions::from_mode(0o000)).unwrap();
        let root = t.to_str().unwrap();
        assert_eq!(proposal_dirs(1234, root).unwrap(), vec![t.join("FXE/202230/p001234")]);
        // nothing matched and an error occurred: Unavail, not "absent"
        assert_eq!(proposal_dirs(1235, root), Err(Fail::Unavail));
        std::fs::set_permissions(t.join("AAA"), std::fs::Permissions::from_mode(0o755)).unwrap();
        let _ = std::fs::remove_dir_all(&t);
    }
}
