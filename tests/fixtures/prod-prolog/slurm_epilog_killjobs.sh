#!/bin/bash
#
# This script will kill any user processes on a node when the last
# SLURM job there ends. For example, if a user directly logs into
# an allocated node SLURM will not kill that process without this
# script being executed as an epilog.
#
# SLURM_BIN can be used for testing with private version of SLURM
#SLURM_BIN="/usr/bin/"
#


if [ x$SLURM_UID = "x" ] ; then
	return 0
fi
if [ x$SLURM_JOB_ID = "x" ] ; then
	return 0
fi

#
# Don't try to kill user root or system daemon jobs
#
if [ $SLURM_UID -lt 100 ] ; then
	return 0
fi

job_list=`${SLURM_BIN}squeue --noheader --format=%A --user=$SLURM_UID --node=localhost`
for job_id in $job_list
do
	if [ $job_id -ne $SLURM_JOB_ID ] ; then
		return 0
	fi
done

#
# No other SLURM jobs, purge all remaining processes of this user
#
pkill -KILL -U $SLURM_UID

#
#  clean up /tmp and /scratch/ as well
#
find /tmp/ /var/tmp/ /scratch/ /dev/shm -user $SLURM_UID -exec rm -rf {} \;

if [ -z $SLURM_JOB_USER ] ; then
    return 0
fi

#
# remove leftover allocated shared memory
#
for ID in $(/usr/bin/ipcs -m | grep $SLURM_JOB_USER |  awk '$2 ~ /[0-9]+/ {print $2}'); do
    /usr/bin/ipcrm -m $ID
done

#
# remove leftover allocated semaphores
#
for ID in $(/usr/bin/ipcs -s | grep $SLURM_JOB_USER |  awk '$2 ~ /[0-9]+/ {print $2}'); do
    /usr/bin/ipcrm -s $ID
done

#
# remove leftover allocated message queues
#
for ID in $(/usr/bin/ipcs -q | grep $SLURM_JOB_USER |  awk '$2 ~ /[0-9]+/ {print $2}'); do
    /usr/bin/ipcrm -q $ID
done

return 0
