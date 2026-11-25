#!/bin/bash
# Sanitised copy of the production epilog (cleandocker is disabled there, unmountnfs is NFS-only).
exec >>/var/log/slurm/epilog-test.$SLURM_JOB_USER.log 2>&1
set -x

source /etc/slurm/slurm_epilog_killjobs.sh

exit 0
