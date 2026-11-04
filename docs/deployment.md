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

## Installed by the package (do not template, or use `noreplace`-safe overrides)

| File | Notes |
|------|-------|
| `/usr/lib64/libnss_proposal.so.2` | The NSS module. |
| `/etc/proposal-nss.conf` | `%config(noreplace)`. Keys `primary_gid` and `supplementary_gid`, both required. Invalid or missing file means the module exposes no users. |
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
