use libc::uid_t;
use libnss::group::Group;
use libnss::initgroups::InitgroupsHooks;
use libnss::interop::Response;
use libnss::passwd::{Passwd, PasswdHooks};
use libnss::{libnss_initgroups_hooks, libnss_passwd_hooks};

const CONF: &str = "/etc/proposal-nss.conf";

struct Conf {
    primary_gid: u32,
    supp_gid: u32,
}

// `key = value` lines, `#` comments. Both keys required, unknown keys rejected, so a
// typo fails closed (no users) instead of silently handing out wrong groups.
fn parse(text: &str) -> Option<Conf> {
    let (mut primary, mut supp) = (None, None);
    for line in text.lines().map(|l| l.split('#').next().unwrap_or("").trim()).filter(|l| !l.is_empty()) {
        let (k, v) = line.split_once('=')?;
        let v = v.trim().parse().ok();
        match k.trim() {
            "primary_gid" => primary = Some(v?),
            "supplementary_gid" => supp = Some(v?),
            _ => return None,
        }
    }
    Some(Conf { primary_gid: primary?, supp_gid: supp? })
}

// Read per lookup: the file is tiny and this keeps Puppet changes live without a restart.
fn conf() -> Option<Conf> {
    parse(&std::fs::read_to_string(CONF).ok()?)
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
fn exists(no: u32) -> bool {
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
        return rc == 0 && !res.is_null();
    }
}

fn live(no: Option<u32>) -> Option<u32> {
    no.filter(|n| exists(*n))
}

fn name_to_no(name: &str) -> Option<u32> {
    let digits = name.strip_prefix("euxfel-p")?;
    if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

fn passwd(no: u32, c: &Conf) -> Passwd {
    Passwd {
        name: format!("euxfel-p{no:06}"),
        passwd: "*".into(), // locked; "x" makes pam_unix look up a shadow entry we don't provide
        uid: id_of(no),
        gid: c.primary_gid,
        gecos: format!("p{no:06} user"),
        dir: "/nonexistent".into(), // unspecified, change if homes are needed
        shell: "/sbin/nologin".into(),
    }
}

// initgroups only reads the gid; the real entries live in other NSS sources.
fn member(gid: u32) -> Group {
    Group { name: String::new(), passwd: "x".into(), gid, members: vec![] }
}

fn found<T>(v: Option<T>) -> Response<T> {
    v.map_or(Response::NotFound, Response::Success)
}

struct ProposalPasswd;
libnss_passwd_hooks!(proposal, ProposalPasswd);
impl PasswdHooks for ProposalPasswd {
    // no enumeration of 1M synthetic users
    fn get_all_entries() -> Response<Vec<Passwd>> {
        Response::Success(vec![])
    }
    fn get_entry_by_uid(uid: uid_t) -> Response<Passwd> {
        let c = conf();
        found(c.as_ref().and_then(|c| live(id_to_no(uid)).map(|n| passwd(n, c))))
    }
    fn get_entry_by_name(name: String) -> Response<Passwd> {
        let c = conf();
        found(c.as_ref().and_then(|c| live(name_to_no(&name)).map(|n| passwd(n, c))))
    }
}

struct ProposalInitgroups;
libnss_initgroups_hooks!(proposal, ProposalInitgroups);
impl InitgroupsHooks for ProposalInitgroups {
    fn get_entries_by_user(user: String) -> Response<Vec<Group>> {
        let c = conf();
        found(c.as_ref().and_then(|c| live(name_to_no(&user)).map(|n| vec![member(id_of(n)), member(c.supp_gid)])))
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
        let p = passwd(900_511, &c);
        assert_eq!((p.name.as_str(), p.uid, p.gid), ("euxfel-p900511", 609_005_112, 15355));
        assert_eq!(p.gecos, "p900511 user");
        assert_eq!(name_to_no(&p.name), Some(900_511));
        for bad in ["euxfel-p1234", "euxfel-p1000000", "euxfel-p+12345", "euxfel-p00123x", "root"] {
            assert_eq!(name_to_no(bad), None, "{bad}");
        }
    }
}
