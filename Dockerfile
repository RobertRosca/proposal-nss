# Test image: builds the module against the container's glibc, installs it next to a
# single-node Slurm (Debian packages, munge auth, no accounting DB), then runs the integration tests via cargo-nextest (test/entrypoint.sh).
FROM rust:1-trixie
RUN cargo install --locked cargo-nextest
RUN apt-get update && apt-get install -y --no-install-recommends slurm-wlm munge sudo \
 && rm -rf /var/lib/apt/lists/* \
 && groupadd -g 3555 exfel && groupadd -g 15355 exfel_prop_syn \
 && useradd -M -u 55635 -g 3555 xdamnprd \
 && useradd -M -u 55636 -g 3555 otheruser
WORKDIR /src
COPY . .
RUN cargo build --release && cargo test --release --no-run \
 && install -m644 target/release/libnss_proposal.so /usr/lib/libnss_proposal.so.2 \
 && install -m440 sudoers/proposal-spawner /etc/sudoers.d/proposal-spawner \
 && sed -i -E 's/^passwd:.*/& proposal/' /etc/nsswitch.conf \
 && echo 'initgroups: files proposal' >> /etc/nsswitch.conf
ENTRYPOINT ["bash", "/src/test/entrypoint.sh"]
