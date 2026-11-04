#!/bin/bash
# Container entrypoint: starts munge+slurmctld+slurmd, creates the external group fixture, runs the tests.
set -euo pipefail

# start munge + slurmctld + slurmd, wait for idle node
host=$(hostname -s)

install -d -o munge -g munge /run/munge /var/log/munge
runuser -u munge -- munged
install -d -o slurm -g slurm /var/spool/slurmctld /var/log/slurm
install -d /var/spool/slurmd
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
SelectType=select/cons_tres
SelectTypeParameters=CR_Core
SlurmdParameters=config_overrides
NodeName=$host CPUs=4 Sockets=1 CoresPerSocket=4 ThreadsPerCore=1 RealMemory=512 State=UNKNOWN
PartitionName=debug Nodes=$host Default=YES MaxTime=5 State=UP
CONF
echo CgroupPlugin=disabled > /etc/slurm/cgroup.conf
slurmctld
slurmd
for _ in $(seq 30); do sinfo -h -o %T 2>/dev/null | grep -q idle && break; sleep 1; done
sinfo -h -o %T | grep -q idle || { sinfo; tail -n 30 /var/log/slurm/*.log; exit 1; }

# the proposal group is managed elsewhere: 1234 has one, 1235 does not
groupadd -g 600012342 60001234-part

# extra args pass through, e.g. `-E 'not binary(slurm)'`; JUnit goes to target/nextest/ci/junit.xml
cd /src
exec cargo nextest run --profile ci --release "$@"
