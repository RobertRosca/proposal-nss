use libc::{gid_t, uid_t};
use libnss::group::{Group, GroupHooks};
use libnss::initgroups::InitgroupsHooks;
use libnss::interop::Response;
use libnss::passwd::{Passwd, PasswdHooks};
use libnss::{libnss_group_hooks, libnss_initgroups_hooks, libnss_passwd_hooks};

const BASE: u32 = 600_000_000;
// GID of the `exfel_prop_syn` group (primary), and of `exfel` (supplementary, via initgroups).
const PRIMARY_GID: u32 = 15355;
const SUPP_GID: u32 = 3555;

fn uid_to_no(uid: u32) -> Option<u32> {
    uid.checked_sub(BASE).filter(|n| *n < 1_000_000)
}

// Group gid is `60{no:06}2`, i.e. 600_000_002 + 10 * no.
fn gid_to_no(gid: u32) -> Option<u32> {
    let d = gid.checked_sub(600_000_002).filter(|d| d % 10 == 0)?;
    Some(d / 10).filter(|n| *n < 1_000_000)
}

fn name_to_no(name: &str) -> Option<u32> {
    six_digits(name.strip_prefix("euxfel-p")?)
}

fn group_name_to_no(name: &str) -> Option<u32> {
    six_digits(name.strip_prefix("60")?.strip_suffix("-part")?)
}

fn six_digits(digits: &str) -> Option<u32> {
    if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

fn passwd(no: u32) -> Passwd {
    Passwd {
        name: format!("euxfel-p{no:06}"),
        passwd: "x".into(),
        uid: BASE + no,
        gid: PRIMARY_GID,
        gecos: format!("p{no:06} user"),
        dir: "/nonexistent".into(), // unspecified, change if homes are needed
        shell: "/sbin/nologin".into(),
    }
}

fn group(no: u32) -> Group {
    Group {
        name: format!("60{no:06}-part"),
        passwd: "x".into(),
        gid: 600_000_002 + 10 * no,
        members: vec![],
    }
}

// name only; the real `exfel` entry is resolved by the other NSS sources
fn exfel() -> Group {
    Group { name: "exfel".into(), passwd: "x".into(), gid: SUPP_GID, members: vec![] }
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
        found(uid_to_no(uid).map(passwd))
    }
    fn get_entry_by_name(name: String) -> Response<Passwd> {
        found(name_to_no(&name).map(passwd))
    }
}

struct ProposalGroup;
libnss_group_hooks!(proposal, ProposalGroup);
impl GroupHooks for ProposalGroup {
    fn get_all_entries() -> Response<Vec<Group>> {
        Response::Success(vec![])
    }
    fn get_entry_by_gid(gid: gid_t) -> Response<Group> {
        found(gid_to_no(gid).map(group))
    }
    fn get_entry_by_name(name: String) -> Response<Group> {
        found(group_name_to_no(&name).map(group))
    }
}

struct ProposalInitgroups;
libnss_initgroups_hooks!(proposal, ProposalInitgroups);
impl InitgroupsHooks for ProposalInitgroups {
    fn get_entries_by_user(user: String) -> Response<Vec<Group>> {
        found(name_to_no(&user).map(|n| vec![group(n), exfel()]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapping() {
        assert_eq!(uid_to_no(599_999_999), None);
        assert_eq!(uid_to_no(600_000_000), Some(0));
        assert_eq!(uid_to_no(600_999_999), Some(999_999));
        assert_eq!(uid_to_no(601_000_000), None);
        let p = passwd(1234);
        assert_eq!((p.name.as_str(), p.uid, p.gid), ("euxfel-p001234", 600_001_234, 15355));
        assert_eq!(p.gecos, "p001234 user");
        let g = group(900_511);
        assert_eq!((g.name.as_str(), g.gid), ("60900511-part", 609_005_112));
        assert_eq!(gid_to_no(609_005_112), Some(900_511));
        assert_eq!(gid_to_no(609_005_111), None);
        assert_eq!(gid_to_no(600_000_001), None);
        assert_eq!(group_name_to_no(&g.name), Some(900_511));
        assert_eq!(group_name_to_no("60900511"), None);
        assert_eq!(name_to_no(&p.name), Some(1234));
        for bad in ["euxfel-p1234", "euxfel-p1000000", "euxfel-p+12345", "euxfel-p00123x", "root"] {
            assert_eq!(name_to_no(bad), None, "{bad}");
        }
    }
}
