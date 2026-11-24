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
<br><br>
- Easiest option would be a per-proposal user
  - Can't register per-proposal users in LDAP
<br><br>
- `proposal-sandbox` emulates a per-proposal user
  - setuid/`cap_setgid` binaries, JWT, SPANK plugin
<br><br>
- This approach makes the per-proposal user 'real' instead
  - Exists only on the hosts, via NSS
  - Kernel, sudo and Slurm treat it as a normal user

</v-clicks>

---

## Goals

<br>

<v-clicks>

- Per-proposal user, without registering anything in LDAP<br><br>

- Persistent across jobs
  - `slurm` jobs, including nested ones, run as the same user<br><br>

- No privileged code of our own
  - No setuid binaries, no tokens, no SPANK plugin<br><br>

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
  - Supplementary groups `exfel` and `60{no:06}-part`<br><br>

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
    - `passwd` and `initgroups` lookups<br><br>
2. Service user can switch to them
    - `sudo` rule on `exfel_prop_syn`<br><br>
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
<br><br>
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
# require_exp_dir = true
```

</v-click>

---
layout: two-cols-header
---

## Method - Code

::left::

```rust
fn get_entry_by_name(name: String) -> Response<Passwd> {
    let c = conf();
    found(c.as_ref().and_then(|c| {
        live(name_to_no(&name), c).map(|n| passwd(n, c))
    }))
}

// user exists only if the LDAP proposal group does
fn live(no: Option<u32>, c: &Conf) -> Option<u32> {
    no.filter(|n| exists(*n) && (!c.require_exp_dir
        || proposal_dirs(*n, &c.exp_dir).next().is_some()))
}
```

::right::

```rust
fn passwd(no: u32, c: &Conf) -> Passwd {
    Passwd {
        name: format!("euxfel-p{no:06}"),
        passwd: "*".into(),
        uid: id_of(no), // 60{no:06}2
        gid: c.primary_gid,
        gecos: format!("p{no:06} user"),
        dir: home(no, &c.exp_dir),
        shell: "/sbin/nologin".into(),
    }
}
```

---
layout: two-cols
---

### Example

<br>

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
<br><br>
- `AllowGroups`
  - `exfel_prop_syn` or `exfel` alone deny proposal users
  - Append `exfel_prop_syn` to the existing groups
  - Relies on an undocumented slurmctld fallback

</v-clicks>

::right::

<v-clicks>

- `require_exp_dir`
  - Only on nodes that mount the exp tree
  - Job without a proposal directory is rejected at submit
  - Directory gone before launch: job requeued and held
<br><br>
- Prolog/epilog
  - `mkhomedir_helper` creates the home in `scratch`
  - Epilog cleans up files and processes of the synthetic uid

</v-clicks>

---

## Trade-offs

<br>

<v-clicks>

- NSS module and config on every host that sees the users
  - Including slurmctld<br><br>

- uid range has to be reserved
  - `600000002` to `609999992`, the proposal group gids
  - No real user may be allocated a uid in it

</v-clicks>

---
layout: center
---

# Questions?
