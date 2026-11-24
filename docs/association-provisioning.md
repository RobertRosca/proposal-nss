# Slurm association provisioning

With `AccountingStorageEnforce=associations,limits` and `AllowNoDefAcct=no`, every proposal user needs
an association before it can submit jobs. Without one `sbatch` fails with `Invalid account or
account/partition combination specified`.

## Creating associations

The service that creates a proposal's LDAP group and directory should also create the association:

```sh
sacctmgr -i add user name=euxfel-pNNNNNN account=exfel
```

`sacctmgr` resolves the name through NSS, so the host it runs on needs the module installed.

The order matters:

1. Create the proposal group in LDAP.
1. Check that `getent passwd euxfel-pNNNNNN` works on the slurmctld host (which needs
   `require_prop_dir = false` if it does not mount the exp tree, see [deployment.md](deployment.md)).
1. Run `sacctmgr add`.

If the slurmctld host could not resolve the user when the association was added, the association stays
unusable until `scontrol reconfigure`.

The privileged spawner can also create the association lazily, as a fallback.
