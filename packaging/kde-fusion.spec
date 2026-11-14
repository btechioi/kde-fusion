# Fedora / openSUSE packaging for KDE Fusion.
# Build with: rpmbuild -bb packaging/kde-fusion.spec (after placing the
# source tarball in ~/rpmbuild/SOURCES).

Name:           kde-fusion
Version:        0.1.25
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

# Install the Plasma widget.
mkdir -p %{buildroot}%{_datadir}/plasma/plasmoids/org.kde.plasma.kdefusion
cp -a plasmoid/. %{buildroot}%{_datadir}/plasma/plasmoids/org.kde.plasma.kdefusion/

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
%{_datadir}/plasma/plasmoids/org.kde.plasma.kdefusion/
%{_datadir}/applications/kdefusion.desktop
%{_datadir}/icons/hicolor/scalable/apps/kdefusion.svg
%{_datadir}/icons/hicolor/symbolic/apps/kdefusion-symbolic.svg
%{_userunitdir}/kdefusiond.service

%changelog
* Sat Nov 07 2026 KDE Fusion contributors - 0.1.25-1
- Fix Moonlight video: detect WAYLAND_DISPLAY for daemon started outside graphical session.

* Sat Nov 07 2026 KDE Fusion contributors - 0.1.24-1
- Use Moonlight auto video decoder (HW when available, no hard errors).

* Sat Nov 07 2026 KDE Fusion contributors - 0.1.23-1
- Fix codec selection: default to H.264 when viewer HEVC decode unknown.

* Sat Nov 07 2026 KDE Fusion contributors - 0.1.22-1
- Fix discovery: prefer fresh addresses, drop stale IPs.

* Sat Nov 07 2026 KDE Fusion contributors - 0.1.21-1
- Hardware-accelerate Moonlight; prefer H.264 when viewer lacks HEVC decode.

* Sat Nov 07 2026 KDE Fusion contributors - 0.1.20-1
- Plasmoid: mode/audio/display toggles, peer quick-start.

* Sat Nov 07 2026 KDE Fusion contributors - 0.1.19-1
- Fix plasmoid: fullRepresentation is a PlasmoidItem property, not attached.

* Sat Nov 07 2026 KDE Fusion contributors - 0.1.18-1
- Fix plasmoid: use PlasmoidHeading (Header type missing).

* Sat Nov 07 2026 KDE Fusion contributors - 0.1.17-1
- Install Plasma widget by default; disable tray by default.

* Sat Nov 07 2026 KDE Fusion contributors - 0.1.16-1
- Add Plasma widget (plasmoid) skeleton.

* Sat Nov 07 2026 KDE Fusion contributors - 0.1.15-1
- Add bidirectional clipboard sync (wl-clipboard).

* Sat Nov 07 2026 KDE Fusion contributors - 0.1.14-1
- Fix bottom status bar (larger status dot).

* Sat Nov 07 2026 KDE Fusion contributors - 0.1.13-1
- UI: KDE Connect-style empty state.

* Sat Nov 07 2026 KDE Fusion contributors - 0.1.12-1
- Move About back to sidebar; tray left-click opens Devices.

* Sat Nov 07 2026 KDE Fusion contributors - 0.1.11-1
- Tray: KDE Connect-like menu with quick start per peer.

* Sat Nov 07 2026 KDE Fusion contributors - 0.1.10-1
- Consolidate sidebar: remove About/Quit tabs, add About to footer.

* Sat Nov 07 2026 KDE Fusion contributors - 0.1.9-1
- UI polish: placeholder messages for empty states.

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