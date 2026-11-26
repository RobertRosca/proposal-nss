#!/bin/bash
# nextest run-wrapper for the default (host) profile: runs a test binary in a throwaway user+mount
# namespace where the test-conf build of libnss_proposal.so is on LD_LIBRARY_PATH and nsswitch.conf/group are private
# bind mounts, so no root is needed and nothing on the host changes.
set -euo pipefail
so=$(realpath -e "$(dirname "$0")/../../target/test-conf/debug/libnss_proposal.so") # build-module in .config/nextest.toml
t=$(mktemp -d); trap 'rm -rf "$t"' EXIT
mkdir "$t/lib"; ln -s "$so" "$t/lib/libnss_proposal.so.2"
printf 'passwd: files proposal\ngroup: files proposal\n' > "$t/nsswitch.conf"
# conf: shipped file with require_prop_dir off, so tests see users without a proposal dir (tests opt in)
# groups that exist outside this module in production; 1234 has a proposal group, 1235 does not
cp /etc/group "$t/group"
printf 'exfel:x:3555:\nexfel_prop_syn:x:15355:\n60001234-part:x:600012342:\n' >> "$t/group"
mkdir "$t/up" "$t/work"
T="$t" C="$(realpath -e "$(dirname "$0")/../../packaging/etc/proposal-nss.conf")" LD_LIBRARY_PATH="$t/lib" unshare -Urm bash -c \
  'mount -t overlay overlay -o lowerdir=/etc,upperdir=$T/up,workdir=$T/work /etc && cp "$C" /etc/proposal-nss.conf && echo require_prop_dir = false >> /etc/proposal-nss.conf && mount --bind "$1" /etc/nsswitch.conf && mount --bind "$2" /etc/group && shift 2 && exec "$@"' \
  -- "$t/nsswitch.conf" "$t/group" "$@"
