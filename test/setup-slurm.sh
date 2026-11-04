#!/bin/bash
# nextest setup script: starts munge + MariaDB + slurmdbd + slurmctld + slurmd and waits for an idle node.
set -euo pipefail
host=$(hostname -s)


install -d -o munge -g munge /run/munge /var/log/munge
runuser -u munge -- munged
install -d -o slurm -g slurm /var/spool/slurmctld /var/log/slurm
install -d /var/spool/slurmd
# Production uses proctrack/cgroup + task/cgroup,task/affinity; not possible in an unprivileged container:
# slurmd needs a systemd scope (no dbus) and /sys/fs/cgroup is read-only (IgnoreSystemd=yes then
# fails with 'unable to create cgroup /sys/fs/cgroup/system: Read-only file system').
# task/affinity fails too: 'task_g_set_affinity: Operation not permitted' (job never starts).
cat > /etc/slurm/slurm.conf <<CONF
ClusterName=test
SlurmctldHost=$host
SlurmUser=slurm
AuthType=auth/munge
StateSaveLocation=/var/spool/slurmctld
SlurmdSpoolDir=/var/spool/slurmd
SlurmctldLogFile=/var/log/slurm/slurmctld.log
SlurmdLogFile=/var/log/slurm/slurmd.log
ProctrackType=proctrack/linuxproc
TaskPlugin=task/none
SelectType=select/linear
MinJobAge=90
UsePAM=0
EnforcePartLimits=ANY
SlurmdParameters=config_overrides
LaunchParameters=disable_send_gids
GroupUpdateTime=1
AccountingStorageType=accounting_storage/slurmdbd
AccountingStorageHost=localhost
AccountingStorageEnforce=associations,limits
NodeName=$host CPUs=4 Sockets=1 CoresPerSocket=4 ThreadsPerCore=1 RealMemory=512 State=UNKNOWN
PartitionName=debug Nodes=$host Default=YES MaxTime=5 State=UP
PartitionName=syn Nodes=$host MaxTime=5 State=UP AllowGroups=exfel_prop_syn
PartitionName=sup Nodes=$host MaxTime=5 State=UP AllowGroups=exfel
PartitionName=both Nodes=$host MaxTime=5 State=UP AllowGroups=exfel,exfel_prop_syn
PartitionName=acct Nodes=$host MaxTime=5 State=UP AllowAccounts=exfel
PartitionName=lim Nodes=$host MaxTime=5 State=UP QOS=joblim OverSubscribe=FORCE:4
CONF
echo CgroupPlugin=disabled > /etc/slurm/cgroup.conf
# Accounting as in production: AccountingStorageEnforce=associations,limits, slurmdbd AllowNoDefAcct=no.
install -d -o mysql -g mysql /run/mysqld
setsid mariadbd --user=mysql > /var/log/mariadb.log 2>&1 &
for _ in $(seq 30); do mariadb-admin ping --silent 2>/dev/null && break; sleep 1; done
mariadb -e "create database slurm_acct_db; create user 'slurm'@'localhost' identified by 'slurm'; grant all on slurm_acct_db.* to 'slurm'@'localhost';"
cat > /etc/slurm/slurmdbd.conf <<CONF
AuthType=auth/munge
DbdHost=localhost
SlurmUser=slurm
StorageType=accounting_storage/mysql
StorageUser=slurm
StoragePass=slurm
StorageLoc=slurm_acct_db
LogFile=/var/log/slurm/slurmdbd.log
PidFile=/run/slurmdbd.pid
AllowNoDefAcct=no
CONF
chown slurm:slurm /etc/slurm/slurmdbd.conf; chmod 600 /etc/slurm/slurmdbd.conf
slurmdbd
for _ in $(seq 30); do sacctmgr -n show cluster >/dev/null 2>&1 && break; sleep 1; done
sacctmgr -i add cluster test
sacctmgr -i add account exfel
sacctmgr -i add account other
sacctmgr -i add qos joblim
sacctmgr -i modify qos joblim set MaxJobsPerUser=1
# Real users and one proposal user; the sacctmgr calls resolve names through NSS (incl. the module).
sacctmgr -i add user otheruser xdamnprd euxfel-p001234 account=exfel
sacctmgr -i add user outsider account=other
slurmctld
slurmd
for _ in $(seq 30); do sinfo -h -o %T 2>/dev/null | grep -q idle && break; sleep 1; done
sinfo -h -o %T | grep -q idle || { sinfo; tail -n 30 /var/log/slurm/*.log; exit 1; }
