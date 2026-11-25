---
theme: unicorn
layout: center
---

# EuXFEL Proposal NSS

---

## Background

<v-clicks>

- Same problem as `proposal-sandbox`
  - DAMNIT service runs 'untrusted' user code
  - Code should not do more than a user in that proposal could do
- Easiest option would be a per-proposal user
  - Can't register per-proposal users in LDAP
- `proposal-sandbox` emulates a per-proposal user
  - setuid/`cap_setgid` binaries, JWT, SPANK plugin
- This approach makes the per-proposal user 'real' instead
  - Exists only on the hosts, via NSS
  - Kernel, sudo and Slurm treat it as a normal user

</v-clicks>

---
hide: true
---

## Goals

<v-clicks>

- Per-proposal user, without registering anything in LDAP

- Persistent across jobs
  - `slurm` jobs, including nested ones, run as the same user

- No privileged code of our own
  - No setuid binaries, no tokens, no SPANK plugin

- No 'unexpected' behaviour
  - Users write code as if it is running as themselves

</v-clicks>

---
layout: two-cols-header
---

## Method - Description

::left::

<v-clicks>

- One synthetic user per proposal
  - Name `euxfel-p{no:06}`
  - uid is the proposal group's gid, `60{no:06}2`
  - Primary group `exfel_prop_syn`
  - Supplementary groups `exfel` and `60{no:06}-part`

- Exists only while the proposal group exists
  - Group is still managed in LDAP
  - Optionally only while the proposal directory exists

</v-clicks>

::right::

<v-click>
This requires:<br><br>
</v-click>

<v-clicks>

1. NSS module to resolve the users
    - `passwd` and `initgroups` lookups
2. Service user can switch to them
    - `sudo` rule on `exfel_prop_syn`
3. Slurm accepts them
    - Same NSS module on slurmctld and compute nodes

</v-clicks>

::bottom::

<v-click>

<p class="text-xs opacity-75">
<code>exfel_prop_syn</code> marks the synthetic users: sudo matches it as a Runas group (user names don't take wildcards), and slurmctld's <code>AllowGroups</code> check compares the primary group name. Keep it free of real users.
</p>

</v-click>

---
layout: two-cols-header
---

## Method - Overview

::left::

<v-clicks>

- `libnss_proposal.so.2`:
  - `passwd` by name or uid
  - `initgroups` adds the proposal group and `exfel`
  - Checks the proposal group exists with `getgrgid`
  - No enumeration of 1M users
  - Reads `/etc/proposal-nss.conf` on every lookup
- `/etc/sudoers.d/proposal-spawner`:
  - Only `xdamnprd` can switch to proposal users
  - Matched by primary group, since user names don't take wildcards

</v-clicks>

::right::

<v-click>

```
# /etc/nsswitch.conf
passwd:     files sss systemd proposal
group:      files sss systemd proposal
```

</v-click>

<v-click>

```
# /etc/sudoers.d/proposal-spawner
xdamnprd ALL=(%exfel_prop_syn) NOPASSWD: ALL
```

</v-click>

<v-click>

```ini
# /etc/proposal-nss.conf
primary_gid = 15355      # exfel_prop_syn
supplementary_gid = 3555 # exfel
# exp_dir = /gpfs/exfel/exp
# require_prop_dir = false # hosts without the exp tree
```

</v-click>

---
layout: two-cols-header
---

## Method - Code

::left::

```rust
fn get_entry_by_name(name: String) -> Response<Passwd> {
    lookup(name_to_no(&name), true, passwd)
}

// user exists only if the LDAP proposal group does
// (and, with require_prop_dir, the proposal dir)
fn live(no: u32, c: &Conf, need_dirs: bool)
    -> Result<Option<Vec<PathBuf>>, Fail> {
    if !exists(no)? { return Ok(None); }
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
```

::right::

```rust
fn passwd(no: u32, c: &Conf, dirs: Vec<PathBuf>) -> Passwd {
    Passwd {
        name: format!("euxfel-p{no:06}"),
        passwd: "*".into(),
        uid: id_of(no), // 60{no:06}2
        gid: c.primary_gid,
        gecos: format!("p{no:06} user"),
        dir: home(dirs), // {exp_dir}/*/*/p{no}/scratch/...
        shell: "/sbin/nologin".into(),
    }
}
```

---
layout: two-cols
---

### Example

<v-clicks>

1. Proposal user resolves through NSS
    - Home is under the proposal's `scratch`
2. `xdamnprd` switches to it with `sudo`
3. Groups come from `initgroups`

</v-clicks>

<br>

<v-clicks>

4. Jobs submitted from inside a job
    - Run as the same user
    - No plugin, Slurm just sees a normal uid

</v-clicks>

::right::

````md magic-move {at:1}
```
```

```shellsession
[xdamnprd@test]$ getent passwd euxfel-p001234
euxfel-p001234:*:600012342:15355:p001234 user:
  /gpfs/exfel/exp/<inst>/<cycle>/p001234/scratch/
  .exfl_prop_syn-home:/sbin/nologin
```

```shellsession
[xdamnprd@test]$ getent passwd euxfel-p001234
euxfel-p001234:*:600012342:15355:p001234 user:
  /gpfs/exfel/exp/<inst>/<cycle>/p001234/scratch/
  .exfl_prop_syn-home:/sbin/nologin

[xdamnprd@test]$ sudo -u euxfel-p001234 id
uid=600012342(euxfel-p001234) gid=15355(exfel_prop_syn) ...
```

```shellsession
[xdamnprd@test]$ getent passwd euxfel-p001234
euxfel-p001234:*:600012342:15355:p001234 user:
  /gpfs/exfel/exp/<inst>/<cycle>/p001234/scratch/
  .exfl_prop_syn-home:/sbin/nologin

[xdamnprd@test]$ sudo -u euxfel-p001234 id
uid=600012342(euxfel-p001234) gid=15355(exfel_prop_syn) ...

[xdamnprd@test]$ sudo -u euxfel-p001234 id -G
15355 3555 600012342
```

```shellsession
[xdamnprd@test]$ getent passwd euxfel-p001234
euxfel-p001234:*:600012342:15355:p001234 user:
  /gpfs/exfel/exp/<inst>/<cycle>/p001234/scratch/
  .exfl_prop_syn-home:/sbin/nologin

[xdamnprd@test]$ sudo -u euxfel-p001234 id
uid=600012342(euxfel-p001234) gid=15355(exfel_prop_syn) ...

[xdamnprd@test]$ sudo -u euxfel-p001234 id -G
15355 3555 600012342

# outer job, as euxfel-p001234, submits an inner job
outer uid=600012342 user=euxfel-p001234
UserId=euxfel-p001234(600012342)
inner uid=600012342 user=euxfel-p001234
```

````

---
layout: two-cols-header
---

### Slurm

::left::

<v-clicks>

- Accounting
  - Each proposal user needs an association
  - `sacctmgr add user name=euxfel-p001234 account=exfel`
  - Per-user QOS limits apply per proposal
- `AllowGroups`
  - `exfel_prop_syn` or `exfel` alone deny proposal users
  - Append `exfel_prop_syn` to the existing groups
  - Relies on an undocumented slurmctld fallback

</v-clicks>

::right::

<v-clicks>

- `require_prop_dir` (on by default)
  - Off on hosts without the exp tree (head/login nodes)
  - No proposal directory at submit: rejected (if the submit host has it on)
  - No directory at launch: job requeued and held until `scontrol release`
- Prolog/epilog
  - `mkhomedir_helper` creates the home in `scratch`
  - Epilog cleans up files and processes of the synthetic uid

</v-clicks>

---

## Trade-offs

<v-clicks>

- NSS module and config on every host that sees the users
  - Including slurmctld

- uid range has to be reserved
  - `600000002` to `609999992`, the proposal group gids
  - No real user may be allocated a uid in it

</v-clicks>

---
layout: center
---

# Questions?
