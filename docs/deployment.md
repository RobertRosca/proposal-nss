# Deployment / configuration management

The package installs everything it can itself. A host manager (e.g. Puppet) only has to do the following.

## Must be provided by the site

| What | Why |
|------|-----|
| Groups `exfel` and `exfel_prop_syn`, user `xdamnprd` (directory service) | Proposal users get these as gids; sudoers rule names them. Package does not create them. |
| `/etc/nsswitch.conf` entries (below) | glibc NSS has no drop-in mechanism, so the package cannot edit it. |
| Values in `/etc/proposal-nss.conf` if the GIDs differ from the shipped defaults | Must match `getent group exfel_prop_syn` / `exfel`. |

### nsswitch.conf

Append `proposal` to the existing `passwd` and `group` lines, e.g.:

```
passwd:     files sss systemd proposal
group:      files sss systemd proposal
```

Keep `proposal` after the real sources so real users always win. The module has no group
lookups, only an initgroups hook, which glibc calls from the `group` sources. Do not add an
`initgroups:` line: it replaces the `group` sources for supplementary groups, so a line
without `sss` would strip LDAP users' groups.

If `/etc/nsswitch.conf` is owned by another Puppet module (e.g. `ldap::client`), make the
change there; a separate resource editing the file would be overwritten.

## Slurm partitions with AllowGroups

Proposal users are denied by `AllowGroups=exfel_prop_syn` or `AllowGroups=exfel` on their own.
slurmctld builds its allowed-uid list from group members and a passwd enumeration, neither of
which the module provides, and it never consults supplementary groups.

Append `exfel_prop_syn` to the existing `AllowGroups` of every partition proposal users must reach
(`upex`, `exfel`), keeping the current groups:

```
PartitionName=upex ... AllowGroups=<existing groups>,exfel_prop_syn
```

This works through slurmctld's fallback that compares the user's primary group name with
`AllowGroups` (`validate_group` in `partition_mgr.c`, identical in 24.11.5 and 25.11.5). It needs:

- at least one other listed group that resolves with members on the slurmctld host
  (`getent group <name>` there; an empty allowed-uid list denies immediately);
- `getent group exfel_prop_syn` resolving on the slurmctld host, so the passwd gid maps to a name.

`exfel_prop_syn` alone in `AllowGroups` always denies. The fallback is undocumented behaviour;
re-run `tests/slurm.rs` (`allowgroups_real_plus_synthetic_group_accepts_proposal_user`) against
each new Slurm release. `AllowAccounts` is the alternative if the access semantics may change.

## Installed by the package (do not template, or use `noreplace`-safe overrides)

| File | Notes |
|------|-------|
| `/usr/lib64/libnss_proposal.so.2` | The NSS module. |
| `/etc/proposal-nss.conf` | `%config(noreplace)`. Keys `primary_gid` and `supplementary_gid`, both required; optional `exp_dir` (default `/gpfs/exfel/exp`, see "Home directory") and `require_prop_dir` (default `false`, see "Require proposal directory"). Invalid or missing file means the module exposes no users. |
| `/etc/sudoers.d/proposal-spawner` | `%config(noreplace)`, mode 0440. Validate with `visudo -cf` before deploying a modified copy. |

## Puppet sketch

```puppet
package { 'proposal-nss': ensure => installed }

file { '/etc/proposal-nss.conf':   # only if GIDs differ from the shipped file
  content      => epp('site/proposal-nss.conf.epp'),
  require      => Package['proposal-nss'],
}

# nsswitch.conf: append `proposal` to passwd and group in the module that owns the file
```

No service to restart: nscd/sssd caches may need flushing after changing the config (`nscd -i passwd`).

### Home directory

`pw_dir` is `{exp_dir}/{instrument}/{cycle}/p{no:06}/scratch/.exfl_prop_syn-home`. The uid only
encodes the proposal number, so the module scans `{exp_dir}/*/*/p{no:06}` on every passwd lookup
(no cache; new proposals show up at once). If several proposal directories match, the
lexicographically smallest path wins. If none match (or `exp_dir` is unreadable, e.g. GPFS not
mounted) `pw_dir` is `/nonexistent`; the user itself stays visible, since existence is decided by
the proposal group (unless `require_prop_dir` is set). The module never creates the directory.

### Require proposal directory

Set `require_prop_dir = true` in `/etc/proposal-nss.conf` on hosts that mount the exp tree (the
allocation nodes). A proposal user is then visible, to passwd and initgroups lookups alike, only
while `{exp_dir}/*/*/p{no:06}` exists, so `sudo -u` and job launch fail for a proposal without a
directory and `pw_dir` is never `/nonexistent`. Only that directory is required; the
`scratch/.exfl_prop_syn-home` home is created by `mkhomedir_helper`.

Leave it `false` on hosts without the tree (head and login nodes). With it set there, every proposal
user vanishes, so slurmctld and `sbatch` reject all jobs and the `AllowGroups` fallback above
fails. On a host that has the option set, an unmounted exp tree hides all proposal users.
nscd's negative cache (default 20 s for passwd) delays a new proposal directory becoming usable.
