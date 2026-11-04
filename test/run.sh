#!/bin/bash
# Runs inside the container: starts munge+slurmctld+slurmd, then checks NSS and Slurm behaviour.
set -euo pipefail
U=600012342 N=euxfel-p001234

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

echo "--- NSS"
# the proposal group is managed elsewhere: 1234 has one, 1235 does not
GID=600012342
groupadd -g $GID 60001234-part
getent passwd $U | grep -x "$N:x:$U:15355:p001234 user:/nonexistent:/sbin/nologin"
getent passwd $N >/dev/null
id $N | grep -F "uid=$U($N) gid=15355(exfel_prop_syn)" | grep -F "$GID(60001234-part)" | grep -F "3555(exfel)"
# the module exposes no groups
! getent group 600012352
! getent group 60001235-part
for k in 599999999 610000002 600012341 euxfel-p1234; do ! getent passwd $k || exit 1; done

echo "--- root submits as proposal user; job submits a nested job as itself"
cat > /tmp/outer.sh <<'JOB'
#!/bin/bash
set -e
echo "outer uid=$(id -u) user=$(id -un) groups=$(id -G)"
inner=$(sbatch --parsable --wait --chdir=/tmp --output=/tmp/inner.out --wrap 'echo "inner uid=$(id -u) user=$(id -un)"')
echo "inner jobid=$inner"
scontrol show job "$inner" | grep -o 'UserId=[^ ]*'
JOB
chmod 755 /tmp/outer.sh
touch /tmp/outer.out /tmp/inner.out; chmod 666 /tmp/outer.out /tmp/inner.out
sbatch --uid=$U --gid=15355 --wait --chdir=/tmp --output=/tmp/outer.out /tmp/outer.sh
cat /tmp/outer.out /tmp/inner.out
grep -F "outer uid=$U user=$N" /tmp/outer.out
grep -F "UserId=$N($U)" /tmp/outer.out
grep -F "inner uid=$U user=$N" /tmp/inner.out
echo ALL OK
