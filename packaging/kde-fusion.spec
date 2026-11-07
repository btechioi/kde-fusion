# Fedora / openSUSE packaging for KDE Fusion.
# Build with: rpmbuild -bb packaging/kde-fusion.spec (after placing the
# source tarball in ~/rpmbuild/SOURCES).

Name:           kde-fusion
Version:        0.1.8
Release:        1%{?dist}
Summary:        KDE-native desktop mirroring and extending over Sunshine and Moonlight

License:        GPL-3.0-or-later
URL:            https://example.invalid/kde-fusion
Source0:        %{name}-%{version}.tar.gz

BuildRequires:  cargo
BuildRequires:  rust
BuildRequires:  qt6-qtbase

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

# Bundle a copy of the Qt QML runtime named "kdefusion" so Qt derives the
# application name (and thus the Wayland app id / taskbar icon) from it.
mkdir -p %{buildroot}%{_libdir}/kdefusion
if command -v qml-qt6 >/dev/null 2>&1; then
  install -Dm755 "$(command -v qml-qt6)" %{buildroot}%{_libdir}/kdefusion/kdefusion
elif command -v qml >/dev/null 2>&1; then
  install -Dm755 "$(command -v qml)" %{buildroot}%{_libdir}/kdefusion/kdefusion
fi

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
%{_libdir}/kdefusion/
%{_datadir}/kdefusion/
%{_datadir}/applications/kdefusion.desktop
%{_datadir}/icons/hicolor/scalable/apps/kdefusion.svg
%{_datadir}/icons/hicolor/symbolic/apps/kdefusion-symbolic.svg
%{_userunitdir}/kdefusiond.service

%changelog
* Sat Nov 07 2026 KDE Fusion contributors - 0.1.8-1
- Add unscaled windowed mode (native resolution).

* Sat Nov 07 2026 KDE Fusion contributors - 0.1.7-1
- Fix GUI session start (missing resolutionField id).

* Sat Nov 07 2026 KDE Fusion contributors - 0.1.6-1
- Bundle QML runtime as "kdefusion" for correct taskbar icon.

* Sat Nov 07 2026 KDE Fusion contributors - 0.1.5-1
- Make windowed 720p switch always visible.

* Sat Nov 07 2026 KDE Fusion contributors - 0.1.4-1
- Fix taskbar icon by setting Qt application name.

* Sat Nov 07 2026 KDE Fusion contributors - 0.1.3-1
- Add windowed 720p mode and fix taskbar icon.

* Sat Nov 07 2026 KDE Fusion contributors - 0.1.2-1
- Fix extend positioning for left/top layouts (KScreen rejects negative coords).

* Sat Nov 07 2026 KDE Fusion contributors - 0.1.1-1
- Fix extend mode so outputs no longer overlap/mirror.

* Sat Nov 07 2026 KDE Fusion contributors - 0.1.0-1
- Initial Fedora/openSUSE packaging.