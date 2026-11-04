# Version is overridden by the build target from Cargo.toml.
%{!?pkg_version:%global pkg_version 0.1.0}

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
# /etc/nsswitch.conf ("passwd: ... proposal", "initgroups: files proposal") is managed by Puppet.

%description
glibc NSS module (libnss_proposal.so.2) and the sudoers rule that lets the
spawner user switch to the synthetic proposal users.

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
install -Dm440 sudoers/proposal-spawner %{buildroot}%{_sysconfdir}/sudoers.d/proposal-spawner
install -Dm644 etc/proposal-nss.conf %{buildroot}%{_sysconfdir}/proposal-nss.conf

%check
visudo -cf sudoers/proposal-spawner

%files
%{_libdir}/libnss_proposal.so.2
%config(noreplace) %attr(0440,root,root) %{_sysconfdir}/sudoers.d/proposal-spawner
%config(noreplace) %{_sysconfdir}/proposal-nss.conf

%changelog
* Wed Nov 04 2026 Robert Rosca <32569096+RobertRosca@users.noreply.github.com> - 0.1.0-1
- Initial package
