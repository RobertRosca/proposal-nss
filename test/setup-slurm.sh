#!/bin/bash
# nextest setup script: starts munge + slurmctld + slurmd and waits for an idle node.
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
NodeName=$host CPUs=4 Sockets=1 CoresPerSocket=4 ThreadsPerCore=1 RealMemory=512 State=UNKNOWN
PartitionName=debug Nodes=$host Default=YES MaxTime=5 State=UP
CONF
echo CgroupPlugin=disabled > /etc/slurm/cgroup.conf
slurmctld
slurmd
for _ in $(seq 30); do sinfo -h -o %T 2>/dev/null | grep -q idle && break; sleep 1; done
sinfo -h -o %T | grep -q idle || { sinfo; tail -n 30 /var/log/slurm/*.log; exit 1; }
