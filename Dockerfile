# syntax=docker/dockerfile:1
# Test image: builds the module against the container's glibc, installs it next to a
# single-node Slurm (Debian packages, munge auth, no accounting DB), then runs the
# integration tests via cargo-nextest (see .config/nextest.toml).

ARG RUST_IMAGE=rust:1-trixie

# Stage 1: nextest binary, built once and cached independently of the source tree.
FROM ${RUST_IMAGE} AS nextest
ARG NEXTEST_VERSION=0.9.151
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    cargo install --locked cargo-nextest --version "${NEXTEST_VERSION}"

# Stage 2: system dependencies, fixture users and groups.
FROM ${RUST_IMAGE} AS base
# Keep downloaded .debs so the apt cache mounts are useful.
RUN rm -f /etc/apt/apt.conf.d/docker-clean \
 && echo 'Binary::apt::APT::Keep-Downloaded-Packages "true";' > /etc/apt/apt.conf.d/keep-cache
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked <<EOT
#!/bin/bash
set -euo pipefail
apt-get update
apt-get install -y --no-install-recommends slurm-wlm munge sudo
groupadd -g 3555 exfel
groupadd -g 15355 exfel_prop_syn
useradd -M -u 55635 -g 3555 xdamnprd
useradd -M -u 55636 -g 3555 otheruser
EOT
COPY --from=nextest /usr/local/cargo/bin/cargo-nextest /usr/local/cargo/bin/

# Stage 3: build the module and test binaries, install the module and sudoers.
FROM base AS test
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked <<EOT
#!/bin/bash
set -euo pipefail
cargo build --release
cargo test --release --no-run
install -m644 target/release/libnss_proposal.so /usr/lib/libnss_proposal.so.2
install -m440 sudoers/proposal-spawner /etc/sudoers.d/proposal-spawner
sed -i -E 's/^passwd:.*/& proposal/' /etc/nsswitch.conf
echo 'initgroups: files proposal' >> /etc/nsswitch.conf
EOT
ENTRYPOINT ["cargo", "nextest", "run", "--profile", "ci", "--release"]
