#!/bin/bash
# nextest setup script: cargo only builds the cdylib for `cargo build`, not for tests. The nss tests
# load this test-conf build (conf path from PROPOSAL_NSS_CONF); its own target dir keeps the
# feature out of the regular builds.
set -euo pipefail
cargo build --lib --features test-conf --target-dir "${CARGO_TARGET_DIR:-target}/test-conf"
