# syntax=docker/dockerfile:1
# Test image: builds the module against the container's glibc, installs it next to a
# single-node Slurm (Debian packages, munge auth, slurmdbd+MariaDB accounting), then runs the
# integration tests via cargo-nextest (see .config/nextest.toml).

ARG RUST_IMAGE=rust:1-trixie

# Stage 1: system dependencies, nextest, fixture users and groups.
FROM ${RUST_IMAGE} AS base
ARG NEXTEST_VERSION=0.9.151
# Keep downloaded .debs so the apt cache mounts are useful.
RUN rm -f /etc/apt/apt.conf.d/docker-clean \
 && echo 'Binary::apt::APT::Keep-Downloaded-Packages "true";' > /etc/apt/apt.conf.d/keep-cache
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked <<EOT
#!/bin/bash
set -euo pipefail
apt-get update
apt-get install -y --no-install-recommends slurm-wlm slurmdbd mariadb-server munge sudo
groupadd -g 3555 exfel
groupadd -g 15355 exfel_prop_syn
useradd -M -u 55635 -g 3555 xdamnprd
useradd -M -u 55636 -g 3555 otheruser
useradd -M -u 55637 -g 3555 outsider
groupadd -g 55600 xdata # owns the exp tree in production (p{no} is 0550 xdata:xdata)
useradd -M -u 55600 -g 55600 xdata
EOT
# prebuilt x86_64 glibc binary; use `linux-arm` on aarch64 hosts
RUN curl -LsSf "https://get.nexte.st/${NEXTEST_VERSION}/linux" | tar zxf - -C /usr/local/cargo/bin

# Stage 2: build the module and test binaries, install the module and sudoers.
FROM base AS test
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked <<EOT
#!/bin/bash
set -euo pipefail
cargo build --release
cargo test --release --no-run
# test-conf module for the nss tests (build-module in .config/nextest.toml, a no-op when nextest reruns it)
cargo build --lib --features test-conf --target-dir target/test-conf
install -m644 target/release/libnss_proposal.so /usr/lib/libnss_proposal.so.2
install -m440 etc/sudoers.d/proposal-spawner /etc/sudoers.d/proposal-spawner
install -m644 etc/proposal-nss.conf /etc/proposal-nss.conf
# the container is head node too: proposal users without a dir must resolve (tests opt in to require_prop_dir)
echo 'require_prop_dir = false' >> /etc/proposal-nss.conf
install -m755 tests/fixtures/prod-prolog/*.sh /etc/slurm/
sed -i -E 's/^(passwd|group):.*/& proposal/' /etc/nsswitch.conf
EOT
ENTRYPOINT ["cargo", "nextest", "run", "--profile", "ci", "--release"]
