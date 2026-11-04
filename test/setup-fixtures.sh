#!/bin/bash
# nextest setup script: the proposal group is managed elsewhere: 1234 has one, 1235 does not.
set -euo pipefail
groupadd -g 600012342 60001234-part
# proposals 1240-1244 are used by the accounting tests (tests/slurm.rs); 1235 stays without a group
for n in 1240 1241 1242 1243 1244; do groupadd -g 6000${n}2 6000${n}-part; done
