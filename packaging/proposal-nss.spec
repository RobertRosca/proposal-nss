# Version is overridden by the build target from Cargo.toml.
%{!?pkg_version:%global pkg_version 0.1.1}

Name:           proposal-nss
Version:        %{pkg_version}
Release:        1%{?dist}
Summary:        NSS module synthesising proposal users for sudo/Slurm
License:        LicenseRef-Proprietary
Source0:        %{name}-%{version}.tar.gz
# Output of `cargo vendor`; mock/rpmbuild chroots have no network.
Source1:        %{name}-%{version}-vendor.tar.gz

BuildRequires:  cargo
BuildRequires:  rust
BuildRequires:  sudo
Requires:       sudo

# Site-provided: groups exfel/exfel_prop_syn and user xdamnprd come from the directory service.
# /etc/nsswitch.conf ("passwd: ... proposal", "group: ... proposal") and
# /etc/sudoers.d/proposal-spawner (shipped as %%doc) are managed by Puppet,
# see docs/deployment.md (installed as %%doc).

%description
glibc NSS module (libnss_proposal.so.2) and an example sudoers rule that lets
the spawner user switch to the synthetic proposal users.

%prep
%autosetup
tar xf %{SOURCE1}
mkdir -p .cargo
cat > .cargo/config.toml <<CFG
[source.crates-io]
replace-with = "vendored-sources"
[source.vendored-sources]
directory = "vendor"
CFG

%build
export RUSTFLAGS="%{?build_rustflags}"
cargo build --release --offline --frozen

%install
install -Dm755 target/release/libnss_proposal.so %{buildroot}%{_libdir}/libnss_proposal.so.2
install -Dm644 packaging/etc/proposal-nss.conf %{buildroot}%{_sysconfdir}/proposal-nss.conf

%check
visudo -cf packaging/etc/sudoers.d/proposal-spawner
# the test-conf feature must never reach a release build: sudo would read a user-chosen conf
if grep -aq PROPOSAL_NSS_CONF target/release/libnss_proposal.so; then
  echo "release .so contains the PROPOSAL_NSS_CONF lookup (test-conf feature enabled)" >&2
  exit 1
fi

%files
%doc docs/deployment.md packaging/etc/sudoers.d/proposal-spawner
%{_libdir}/libnss_proposal.so.2
%config(noreplace) %{_sysconfdir}/proposal-nss.conf

%changelog
* Fri Nov 27 2026 Robert Rosca <32569096+RobertRosca@users.noreply.github.com> - 0.1.1-1
- First release (the v0.1.0 tag is reserved by a deleted immutable release)

* Wed Nov 04 2026 Robert Rosca <32569096+RobertRosca@users.noreply.github.com> - 0.1.0-1
- Initial package
