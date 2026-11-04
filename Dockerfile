# Test image: builds the module against the container's glibc, installs it next to a
# single-node Slurm (Debian packages, munge auth, no accounting DB), then runs test/run.sh.
FROM rust:1-trixie
RUN apt-get update && apt-get install -y --no-install-recommends slurm-wlm munge \
 && rm -rf /var/lib/apt/lists/* \
 && groupadd -g 3555 exfel
WORKDIR /src
COPY . .
RUN cargo test --release && cargo build --release \
 && install -m644 target/release/libnss_proposal.so /usr/lib/libnss_proposal.so.2 \
 && sed -i -E 's/^(passwd|group):.*/& proposal/' /etc/nsswitch.conf
CMD ["bash", "/src/test/run.sh"]
