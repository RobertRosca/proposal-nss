#!/bin/bash
# nextest setup script (host profile): cargo only builds the cdylib for `cargo build`, not for tests.
set -euo pipefail
cargo build --lib
