#!/bin/bash
# Sanitised copy of the production prolog: health checks, cpupower, beamline NFS mounts and hmz
# removed; the user-dependent mkhomedir_helper step is verbatim. Trace goes to a file for the tests.
exec >>/var/log/slurm/prolog-test.$SLURM_JOB_USER.log 2>&1
set -x

# create home-dir if missing
[ -d /home/$SLURM_JOB_USER ] || /usr/sbin/mkhomedir_helper $SLURM_JOB_USER

exit 0
