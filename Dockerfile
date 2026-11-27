# syntax=docker/dockerfile:1
# Test image: installs the RPM built here (EL9 glibc) next to a
# single-node Slurm (munge auth, slurmdbd+MariaDB accounting), then runs the integration tests
# via cargo-nextest (see .config/nextest.toml). EL9 and Slurm 25.11 to match the target cluster.

# pinned: a moved tag would rebuild every layer (the Slurm RPMs take ~3 min)
ARG EL_IMAGE=docker.io/almalinux/9-base:9.8-20261114@sha256:9f7dfcd2652774222fe70ef5bfdcecdefdcc6e46cfb1c753c986a46e00ebf5ab
# the target cluster runs 9.8; dnf otherwise resolves $releasever 9 to the latest minor's packages
ARG EL_RELEASE=9.8

# Stage 1: Slurm RPMs from the SchedMD tarball. The cluster's RPMs (DESY desy-common repo) are
# unreachable from outside DESY. Spec defaults (munge, pam, x11, mysql via mariadb-devel) cover what
# the tests use; DESY also builds lua, hwloc, pmix, hdf5, jwt, slurmrestd, rdkafka and cgroupv2.
# This stage does not copy the repo, so its cache survives every source change.
FROM ${EL_IMAGE} AS slurm-rpms
ARG SLURM_VERSION=25.11.5
ARG SLURM_SHA256=c95361df00a035571c101834cd3988c224a38c4a3c8c67252aedb7527d447277
ARG EL_RELEASE
RUN <<EOT
#!/bin/bash
set -euo pipefail
echo "$EL_RELEASE" > /etc/dnf/vars/releasever
dnf -y install dnf-plugins-core rpm-build perl-interpreter bzip2
dnf config-manager --set-enabled crb # munge-devel
t=slurm-${SLURM_VERSION}.tar.bz2
curl -fsSLO "https://download.schedmd.com/slurm/$t"
echo "${SLURM_SHA256}  $t" | sha256sum -c -
tar xjf "$t" "slurm-${SLURM_VERSION}/slurm.spec"
dnf -y builddep "slurm-${SLURM_VERSION}/slurm.spec"
rpmbuild -ta "$t"
# a missing devel package silently drops the plugin instead of failing the build
rpm -qpl /root/rpmbuild/RPMS/x86_64/*.rpm | grep -q accounting_storage_mysql
# keep only the RPMs: the build tree would bloat the layer exported to the GHA cache
rm -rf /root/rpmbuild/{BUILD,SOURCES,SRPMS} "$t" "slurm-${SLURM_VERSION}"
dnf clean all
EOT

# Stage 2: system dependencies, nextest, fixture users and groups.
FROM ${EL_IMAGE} AS base
ARG NEXTEST_VERSION=0.9.151
ARG EL_RELEASE
RUN --mount=type=bind,from=slurm-rpms,source=/root/rpmbuild/RPMS/x86_64,target=/rpms <<EOT
#!/bin/bash
set -euo pipefail
echo "$EL_RELEASE" > /etc/dnf/vars/releasever
# sssd-client: libnss_sss loads and returns UNAVAIL (no sssd), as ahead of the module in production
dnf -y install rust-toolset mariadb-server munge sudo sssd-client hostname \
  /rpms/slurm-[0-9]*.rpm /rpms/slurm-slurmctld-*.rpm /rpms/slurm-slurmd-*.rpm /rpms/slurm-slurmdbd-*.rpm
dnf clean all
useradd -r -M -d /var/lib/slurm -s /sbin/nologin slurm # the Debian packages created it
create-munge-key
mariadb-install-db --user=mysql >/dev/null
install -d /etc/slurm
groupadd -g 3555 exfel
groupadd -g 15355 exfel_prop_syn
useradd -M -u 55635 -g 3555 xdamnprd
useradd -M -u 55636 -g 3555 otheruser
useradd -M -u 55637 -g 3555 outsider
groupadd -g 55600 xdata # owns the exp tree in production (p{no} is 0550 xdata:xdata)
useradd -M -u 55600 -g 55600 xdata
EOT
# prebuilt x86_64 glibc binary; use `linux-arm` on aarch64 hosts
RUN curl -LsSf "https://get.nexte.st/${NEXTEST_VERSION}/linux" | tar zxf - -C /usr/local/bin

# Stage 3: the RPM, built on the shared base so the test stage installs what ships. Export with
#   docker build --target rpms --output type=local,dest=dist .
FROM base AS rpm-build
RUN dnf -y install rpm-build redhat-rpm-config gcc tar gzip && dnf clean all
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
# cargo vendor needs the lib target to exist; the stub is removed before the real sources arrive.
# Own layer: source edits keep the downloaded crates cached.
RUN <<EOT
#!/bin/bash
set -euo pipefail
mkdir src && touch src/lib.rs
cargo vendor -q --locked vendor
rm -r src
EOT
COPY src src
COPY docs docs
COPY packaging packaging
RUN <<EOT
#!/bin/bash
set -euo pipefail
rustc --version
ver=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)
n=proposal-nss-$ver
top=$HOME/rpmbuild; mkdir -p $top/SOURCES
tar czf $top/SOURCES/$n.tar.gz --transform "s,^,$n/," Cargo.toml Cargo.lock src packaging/etc docs
tar czf $top/SOURCES/$n-vendor.tar.gz vendor
rpmbuild -ba --define "pkg_version $ver" packaging/proposal-nss.spec
EOT

# before `test` so a plain build still ends on the test image
FROM scratch AS rpms
COPY --from=rpm-build /root/rpmbuild/RPMS /
COPY --from=rpm-build /root/rpmbuild/SRPMS /srpms

# Stage 4: install the RPM, build the test binaries, add the test-only config.
FROM base AS test
WORKDIR /src
# only what the build and tests read, so edits to docs, CI or packaging specs keep the compile layer
COPY Cargo.toml Cargo.lock ./
COPY .config .config
COPY src src
COPY tests tests
COPY packaging/etc packaging/etc
RUN --mount=type=cache,target=/root/.cargo/registry,sharing=locked <<EOT
#!/bin/bash
set -euo pipefail
cargo test --release --no-run
# test-conf module for the nss tests (build-module in .config/nextest.toml, a no-op when nextest reruns it)
cargo build --lib --features test-conf --target-dir target/test-conf
EOT
# own RUN with a bind mount, so RPM changes (docs, spec) keep the compile layer above
RUN --mount=type=bind,from=rpm-build,source=/root/rpmbuild/RPMS/x86_64,target=/rpms <<EOT
#!/bin/bash
set -euo pipefail
# module and conf come from the package; it ships the sudoers rule only as a %doc (Puppet installs it)
dnf -y install /rpms/proposal-nss-[0-9]*.rpm
dnf clean all
rpm -q proposal-nss
test -f /usr/lib64/libnss_proposal.so.2
test -f /etc/proposal-nss.conf
install -m440 packaging/etc/sudoers.d/proposal-spawner /etc/sudoers.d/proposal-spawner
# the container is head node too: proposal users without a dir must resolve (tests opt in to require_prop_dir)
echo 'require_prop_dir = false' >> /etc/proposal-nss.conf
install -m755 tests/fixtures/prod-prolog/*.sh /etc/slurm/
# production order (maxwell); without a running sssd, sss returns UNAVAIL and lookups continue
sed -i -E 's/^(passwd|group):.*/\1: files sss systemd proposal/' /etc/nsswitch.conf
EOT
ENTRYPOINT ["cargo", "nextest", "run", "--profile", "ci", "--release"]
