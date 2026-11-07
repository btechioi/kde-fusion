# Setup guide

This guide walks through installing KDE Fusion, preparing Sunshine and Moonlight,
and running your first mirrored or extended session. KDE Fusion automates most
of the Sunshine configuration; the manual steps below are only needed for the
hardware and network prerequisites.

## 1. Install the runtime dependencies

KDE Fusion needs Plasma 6 on Wayland plus a small set of tools. Install them
for your distribution before installing KDE Fusion itself.

### Arch Linux / CachyOS

```bash
sudo pacman -S krfb kscreen sunshine moonlight-qt qt6-declarative kirigami \
               libsecret libnotify curl pipewire-pulse
# Intel GPU:
sudo pacman -S intel-media-driver libva-utils
# AMD GPU:
sudo pacman -S libva-mesa-driver libva-utils
```

### Fedora

```bash
sudo dnf install krfb kscreen qt6-qtdeclarative-devel kf6-kirigami libsecret \
                 libnotify curl pipewire-pulseaudio polkit
# Sunshine and Moonlight are installed by KDE Fusion from LizardByte's COPR
# and Flathub (see Diagnostics -> Streaming packages).
# Intel GPU (RPM Fusion free):
sudo dnf install https://mirrors.rpmfusion.org/free/fedora/rpmfusion-free-release-$(rpm -E %fedora).noarch.rpm
sudo dnf install intel-media-driver libva-utils
# AMD GPU:
sudo dnf install mesa-va-drivers
```

### Debian / Ubuntu

```bash
sudo apt install krfb kscreen qml-qt6 qml6-module-org-kde-kirigami \
                 libsecret-tools libnotify-bin curl pipewire-pulse polkitd
# Sunshine and Moonlight are installed by KDE Fusion: krfb from apt,
# Moonlight from Flathub, with a manual step for Sunshine's official .deb.
# Intel GPU:
sudo apt install intel-media-va-driver vainfo
# AMD GPU:
sudo apt install mesa-va-drivers vainfo
```

## 2. Install KDE Fusion

### Arch Linux / CachyOS

```bash
./scripts/make-pkg-source.sh
cd packaging
makepkg -si
systemctl --user enable --now kdefusiond
```

### Fedora / openSUSE

```bash
./scripts/make-pkg-source.sh
rpmbuild -tb kde-fusion-0.1.0.tar.gz
sudo dnf install ~/rpmbuild/RPMS/x86_64/kde-fusion-0.1.0-1.*.rpm
systemctl --user enable --now kdefusiond
```

### Debian / Ubuntu (from source)

```bash
./scripts/install-local.sh
systemctl --user enable --now kdefusiond
```

Make sure `~/.local/bin` is in `PATH`.

## 3. First run

1. Open **KDE Fusion** from the application menu (or run `kdefusion`).
2. Open **Settings** and give each PC a distinct name.
3. Open **Diagnostics** and fix anything marked *Error*.

The **Streaming packages** card detects missing components and installs them:
- Arch: `pkexec pacman -S --needed --noconfirm sunshine moonlight-qt krfb`
- Fedora: `krfb` from dnf, Sunshine from LizardByte's COPR, Moonlight from Flathub
- Debian: `krfb` from apt, Moonlight from Flathub, with a manual step for
  Sunshine's official `.deb`

The card shows the exact command and a live package-manager log. Keep the window
open until it finishes.

## 4. How Sunshine is configured

KDE Fusion writes `~/.config/sunshine/sunshine.conf` automatically when a
session starts. You do not need to use the Sunshine web UI.

```ini
capture = kwin
output_name = Virtual-KDEFusion
encoder = vaapi
stream_audio = enabled   # or disabled, depending on the session audio toggle
```

- `capture = kwin` uses KWin's native screencast protocol, which is the only
  capture method that works with a virtual Wayland output.
- `output_name = Virtual-KDEFusion` makes Sunshine capture the virtual output
  created by `krfb-virtualmonitor` instead of a physical monitor.
- `encoder = vaapi` uses the hardware encoder. KDE Fusion only selects HEVC or
  AV1 when `vainfo` reports encode support; otherwise it falls back to H.264.
- `stream_audio` follows the per-session audio toggle.

Sunshine admin credentials are generated on first run and stored in KWallet /
Secret Service. The web UI is restricted to the local PC
(`origin_web_ui_allowed = pc`); you never need to open it for pairing.

### KWin screencast permission

KWin only allows `zkde_screencast_unstable_v1` to executables with a matching
desktop file. The Sunshine package installs
`/usr/share/applications/dev.lizardbyte.app.Sunshine.kwin.desktop`; if it is
missing, KDE Fusion creates
`~/.local/share/applications/dev.lizardbyte.app.Sunshine.kwin.desktop`
automatically.

## 5. VA-API encoder

Verify that a VA-API encoder is available:

```bash
vainfo
```

Look for `VAEntrypointEncSlice` entries for H.264 or HEVC. If none appear,
install the driver for your GPU:

| GPU | Arch | Fedora | Debian |
| --- | --- | --- | --- |
| Intel | `intel-media-driver` | `intel-media-driver` (RPM Fusion) | `intel-media-va-driver` |
| AMD | `libva-mesa-driver` | `mesa-va-drivers` | `mesa-va-drivers` |
| NVIDIA | `nvidia-vaapi-driver` | `nvidia-vaapi-driver` | `nvidia-vaapi-driver` |

## 6. Audio

KDE Fusion streams system audio through PipeWire/PulseAudio. Make sure a default
sink exists:

```bash
pactl info | grep "Default Sink"
```

If it reports nothing, install `pipewire-pulse` (Arch/Debian) or
`pipewire-pulseaudio` (Fedora) and restart the session.

Audio is a single on/off toggle per session. When off, Moonlight is launched
without an audio stream and Sunshine writes `stream_audio = disabled`. The host
keeps its local audio in v1.

## 7. Network and firewall

The control channel uses TCP port `47811` and is LAN-only. Open it on every
host:

### Arch / CachyOS (firewalld or ufw)

```bash
sudo firewall-cmd --add-port=47811/tcp --permanent && sudo firewall-cmd --reload
# or
sudo ufw allow 47811/tcp
```

### Fedora

```bash
sudo firewall-cmd --add-port=47811/tcp --permanent
sudo firewall-cmd --reload
```

### Debian / Ubuntu

```bash
sudo ufw allow 47811/tcp
```

mDNS discovery (`_kdefusion._tcp.local.`) requires multicast on the LAN. If
discovery does not work, enter the peer's IP address manually in **Devices**.

## 8. Pairing two PCs

1. Start `kdefusiond` on both PCs.
2. On one PC, open **Devices**. The other PC appears under **Nearby** after a
   few seconds.
3. Click **Pair**. A six-digit code is shown on the initiating PC.
4. Approve the request on the other PC. The code must match.
5. Both PCs now store a 256-bit bearer token in KWallet / Secret Service.

Pairing is explicit and one-time. Tokens can be revoked from **Devices**.

## 9. Starting a session

From the peer card or the **Session** page, choose:

- **Mirror**: the virtual output clones the physical display.
- **Extend**: the virtual output is placed at the chosen edge (right, left, top
  or bottom) at the client's native resolution.
- **Headless**: the physical outputs are disabled and only the virtual output
  remains.

When a session starts, KDE Fusion:

1. Creates `Virtual-KDEFusion` through `krfb-virtualmonitor`.
2. Applies the KScreen layout (mirror or extend).
3. Writes the Sunshine configuration and starts the Sunshine user service.
4. Launches Moonlight on the viewer with the right resolution, codec and audio
   settings.
5. Pairs Moonlight with Sunshine using a one-time PIN (first session only).

When the session ends (or after a crash), the previous KScreen layout is
restored and the virtual output is removed.

## 10. Moonlight on the viewer

Moonlight is launched automatically. On Arch it uses the native `moonlight`
binary; on Fedora and Debian it uses the Flatpak
`com.moonlight_stream.Moonlight`, which KDE Fusion starts through
`flatpak run`. No manual Moonlight configuration is required.

If you prefer the AppImage, install it as `moonlight` in `~/.local/bin` and KDE
Fusion will use it.

## 11. CLI

`kdefusionctl` talks to the local daemon over loopback HTTP:

```bash
kdefusionctl status
kdefusionctl peers
kdefusionctl pair <device-id>
kdefusionctl pending
kdefusionctl approve <request-id>
kdefusionctl start --peer <device-id> --mode extend --layout right --audio off
kdefusionctl stop
kdefusionctl doctor
kdefusionctl config
kdefusionctl set streaming.fps 90
kdefusionctl sunshine credentials --username kdefusion --password '...'
kdefusionctl wait --phase streaming --timeout-secs 120
```

Add `--json` to any command for raw output. The API base can be overridden with
`--api` or `KDEFUSION_API`.

## 12. Troubleshooting

### No virtual output appears

`krfb-virtualmonitor` is Wayland-only and creates `Virtual-<name>`.

```bash
kscreen-doctor -o | grep Virtual-
```

If nothing appears, install `krfb` and verify you are in a Plasma Wayland
session (`echo $XDG_SESSION_TYPE`).

### Sunshine does not start

Run Sunshine manually to see its error:

```bash
sunshine
```

Common causes:
- Missing VA-API driver (check `vainfo`).
- Missing KWin screencast permission desktop file.
- Sunshine crashed on startup (reinstall the package).

KDE Fusion captures Sunshine's stderr and shows it in the session error.

### "No route to host"

Open port 47811 on the host (see section 7) and verify the daemon is running:

```bash
ss -ltnp | grep 47811
```

### Moonlight does not connect

The first session pairs Moonlight with Sunshine using a one-time PIN. If
pairing failed, revoke the peer in **Devices** and pair again.

### Session fails with "Virtual-KDEFusion did not become enabled"

Update to a current KDE Fusion build; KScreen omits the `enabled` field for
virtual outputs and recent builds handle this.

### Restoring the display

If a session crashes, KDE Fusion restores the previous KScreen layout on
cleanup. You can also restore manually:

```bash
kscreen-doctor --restore
```

## Further reading

- `docs/TROUBLESHOOTING.md` - symptom-based troubleshooting
- `docs/PROTOCOL.md` - control channel protocol
- `packaging/README.md` - building packages per distribution
