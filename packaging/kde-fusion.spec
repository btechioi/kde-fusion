# Fedora / openSUSE packaging for KDE Fusion.
# Build with: rpmbuild -bb packaging/kde-fusion.spec (after placing the
# source tarball in ~/rpmbuild/SOURCES).

Name:           kde-fusion
Version:        0.1.1
Release:        1%{?dist}
Summary:        KDE-native desktop mirroring and extending over Sunshine and Moonlight

License:        GPL-3.0-or-later
URL:            https://example.invalid/kde-fusion
Source0:        %{name}-%{version}.tar.gz

BuildRequires:  cargo
BuildRequires:  rust

Requires:       krfb
Requires:       kscreen
Requires:       qt6-qtdeclarative-devel
Requires:       kf6-kirigami
Requires:       libsecret
Requires:       libnotify
Requires:       curl
Requires:       polkit
Requires:       pipewire-pulseaudio
Recommends:     intel-media-driver
Recommends:     libva-utils

%description
KDE Fusion mirrors or extends a KDE Plasma Wayland desktop between two
machines using Sunshine and Moonlight, with a native Kirigami frontend.

%prep
%autosetup -n %{name}-%{version}

%build
export CARGO_HOME="${CARGO_HOME:-%{_builddir}/cargo-home}"
cargo build --release --locked --jobs 2

%install
install -Dm755 target/release/kdefusiond %{buildroot}%{_bindir}/kdefusiond
install -Dm755 target/release/kdefusionctl %{buildroot}%{_bindir}/kdefusionctl
install -Dm755 target/release/kdefusion %{buildroot}%{_bindir}/kdefusion

mkdir -p %{buildroot}%{_datadir}/kdefusion
cp -a qml %{buildroot}%{_datadir}/kdefusion/qml

install -Dm644 packaging/kdefusion.desktop %{buildroot}%{_datadir}/applications/kdefusion.desktop
install -Dm644 packaging/kdefusion.svg %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/kdefusion.svg
install -Dm644 packaging/kdefusion-symbolic.svg %{buildroot}%{_datadir}/icons/hicolor/symbolic/apps/kdefusion-symbolic.svg

install -Dm644 systemd/kdefusiond.service %{buildroot}%{_userunitdir}/kdefusiond.service

%files
%license LICENSE
%doc README.md docs/TROUBLESHOOTING.md
%{_bindir}/kdefusiond
%{_bindir}/kdefusionctl
%{_bindir}/kdefusion
%{_datadir}/kdefusion/
%{_datadir}/applications/kdefusion.desktop
%{_datadir}/icons/hicolor/scalable/apps/kdefusion.svg
%{_datadir}/icons/hicolor/symbolic/apps/kdefusion-symbolic.svg
%{_userunitdir}/kdefusiond.service

%changelog
* Sat Nov 07 2026 KDE Fusion contributors - 0.1.1-1
- Fix extend mode so outputs no longer overlap/mirror.

* Sat Nov 07 2026 KDE Fusion contributors - 0.1.0-1
- Initial Fedora/openSUSE packaging.