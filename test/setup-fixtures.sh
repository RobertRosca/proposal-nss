#!/bin/bash
# nextest setup script: the proposal group is managed elsewhere: 1234 has one, 1235 does not.
set -euo pipefail
groupadd -g 600012342 60001234-part
