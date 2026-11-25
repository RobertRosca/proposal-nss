#!/bin/bash
# nextest run-wrapper for the nss binary in the ci profile: the container is already isolated, so
# only put the test-conf build of the module first on the library path. getent/id in the tests then
# honour PROPOSAL_NSS_CONF, while the daemons keep the installed release module.
set -euo pipefail
so=$(realpath -e "${CARGO_TARGET_DIR:-$(dirname "$0")/../target}/test-conf/debug/libnss_proposal.so") # test/build-module.sh
t=$(mktemp -d); trap 'rm -rf "$t"' EXIT
ln -s "$so" "$t/libnss_proposal.so.2"
LD_LIBRARY_PATH="$t" "$@"
