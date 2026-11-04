#!/bin/bash
# nextest run-wrapper for the default (host) profile: runs a test binary in a throwaway user+mount
# namespace where libnss_proposal.so is on LD_LIBRARY_PATH and nsswitch.conf/group are private
# bind mounts, so no root is needed and nothing on the host changes.
set -euo pipefail
bin=$1
so=$(realpath -e "$(dirname "$bin")/../libnss_proposal.so") # cdylib sits next to deps/
t=$(mktemp -d); trap 'rm -rf "$t"' EXIT
mkdir "$t/lib"; ln -s "$so" "$t/lib/libnss_proposal.so.2"
printf 'passwd: files proposal\ngroup: files\ninitgroups: files proposal\n' > "$t/nsswitch.conf"
# groups that exist outside this module in production; 1234 has a proposal group, 1235 does not
cp /etc/group "$t/group"
printf 'exfel:x:3555:\nexfel_prop_syn:x:15355:\n60001234-part:x:600012342:\n' >> "$t/group"
mkdir "$t/up" "$t/work"
T="$t" C="$(realpath -e "$(dirname "$0")/../etc/proposal-nss.conf")" LD_LIBRARY_PATH="$t/lib" unshare -Urm bash -c \
  'mount -t overlay overlay -o lowerdir=/etc,upperdir=$T/up,workdir=$T/work /etc && cp "$C" /etc/proposal-nss.conf && mount --bind "$1" /etc/nsswitch.conf && mount --bind "$2" /etc/group && shift 2 && exec "$@"' \
  -- "$t/nsswitch.conf" "$t/group" "$@"
