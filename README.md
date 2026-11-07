# KDE Fusion

KDE Fusion mirrors or extends a **KDE Plasma 6 Wayland** desktop to another PC
over the LAN. It is a Rust daemon plus a Kirigami/QML frontend, and it drives
Sunshine on the host and Moonlight on the viewer automatically.

Both PCs run a KDE Plasma 6 Wayland distribution (Arch/CachyOS, Fedora or
Debian/Ubuntu are supported). Either PC can be the host or the viewer; the
same package is installed on both.

```
 Host (Plasma Wayland)                              Viewer (Plasma Wayland)
 ┌────────────────────────────┐   control channel   ┌────────────────────────┐
 │ KDE Fusion daemon          │◀───────────────────▶│ KDE Fusion daemon      │
 │  ├─ krfb-virtualmonitor    │  mDNS + TCP + token │  └─ Moonlight          │
 │  ├─ KScreen / kscreen-doctor│                     └────────────────────────┘
 │  └─ Sunshine ──────────────┼──── H.264/HEVC ────▶ Moonlight fullscreen
 └────────────────────────────┘
```

## What it does

- Creates a real KDE virtual output named `Virtual-KDEFusion` through
  `krfb-virtualmonitor`, then captures it with Sunshine's native KWin capture.
- **Mirror** clones the physical display onto the virtual output.
- **Extend** places the virtual output to the right, left, top or bottom of the
  physical display at the client's native resolution.
- **Headless** disables the physical outputs and streams only the virtual one.
- Configures Sunshine automatically (`capture=kwin`, stable `output_name`,
  VA-API encoder, audio on/off) and manages the Sunshine user process.
- Discovers peers with mDNS (`_kdefusion._tcp.local.`), pairs explicitly with a
  six-digit code and a 256-bit bearer token, and launches Moonlight on the
  viewer without opening the Sunshine web UI.
- Stores Sunshine admin credentials and peer tokens in KWallet / Secret Service
- Follows the Plasma color scheme, fonts and icons through Kirigami; the
  launcher loads the KDE platform theme explicitly so this also works when the
  daemon starts the GUI from a systemd user unit
  through `secret-tool`, never in `state.json`.
- Restores the previous KScreen layout when a session ends or after a crash.
- Provides a Kirigami GUI, a tray icon, KDE notifications, a systemd user unit
  and a `kdefusionctl` CLI.

The GUI is QML rendered by the Qt 6 runtime that Plasma already ships. There is
no C++ build step, no CMake and no Ninja requirement.

## Requirements

Both PCs:

- Plasma 6 on **Wayland** (X11 is not supported for virtual outputs)
- `krfb`, `kscreen`, the Qt 6 QML runtime, Kirigami, `libsecret`,
  `libnotify`, `curl`
- `sunshine` on the host, `moonlight` on the viewer (installing both is fine)
- `pipewire-pulse` (Arch/Debian) or `pipewire-pulseaudio` (Fedora/openSUSE) for audio
- A VA-API encoder: `intel-media-driver` on Intel, `libva-mesa-driver`/`mesa-va-drivers`
  on AMD
- `vainfo` from `libva-utils` for diagnostics

KDE Fusion runs on the Arch, Fedora and Debian families.  It detects the
distribution automatically; the **Diagnostics → Streaming packages** card
installs what it can (Arch: pacman; Fedora: Sunshine from LizardByte's COPR
plus Moonlight from Flathub; Debian: krfb from apt plus Moonlight from
Flathub, with a manual step for Sunshine's official .deb).

Install the runtime dependencies for your distribution:

```bash
# Arch / CachyOS
sudo pacman -S krfb kscreen sunshine moonlight-qt qt6-declarative kirigami \
               libsecret libnotify curl pipewire-pulse
# Intel GPU:
sudo pacman -S intel-media-driver libva-utils

# Fedora
sudo dnf install krfb kscreen qt6-qtdeclarative-devel kf6-kirigami libsecret \
                 libnotify curl pipewire-pulseaudio polkit
# Sunshine and Moonlight are installed from Diagnostics (COPR + Flatpak).

# Debian / Ubuntu
sudo apt install krfb kscreen qml-qt6 qml6-module-org-kde-kirigami \
                 libsecret-tools libnotify-bin curl pipewire-pulse polkitd
# Sunshine and Moonlight are installed from Diagnostics (official .deb + Flatpak).
```

## Install

### PKGBUILD (system-wide, recommended)

```bash
# From this repository:
./scripts/make-pkg-source.sh
cd packaging
makepkg -si
```

`makepkg` creates a package that installs the three binaries, the QML tree, the
desktop file, the icon, docs and the systemd user unit. Enable the daemon for
your user session:

```bash
systemctl --user enable --now kdefusiond
```

The unit is ordered after `graphical-session.target` because it needs KWin,
KScreen and the Secret Service.

### User-local install (no root)

```bash
./scripts/install-local.sh
```

This installs to `~/.local` and writes a unit to
`~/.config/systemd/user/kdefusiond.service`. Make sure `~/.local/bin` is in
`PATH`, then:

```bash
systemctl --user enable --now kdefusiond
kdefusion
```

Use `PREFIX=/usr sudo ./scripts/install-local.sh` only if you really want a
manual system-wide install outside pacman.

## First run on both PCs

1. Start `kdefusiond` (the installer enables it) and open **KDE Fusion** from
   the application menu.
2. Open **Settings** and give each PC a distinct name.
3. Open **Diagnostics**. Fix anything marked *Error*. The important checks are:
   - KDE Plasma Wayland session
   - `kscreen-doctor` and `krfb-virtualmonitor`
   - `sunshine` and `moonlight`
   - KWin screencast permission
   - a VA-API encoder
   - a default PipeWire sink

   If Sunshine, Moonlight or krfb are missing, the **Streaming packages**
   card offers **Install missing packages**. It uses Polkit and the system
   package manager; the command and its live log are shown in the same card.
4. On **Settings**, set the Sunshine admin password if Sunshine already had
   credentials. A fresh Sunshine install is bootstrapped automatically.
   Alternatively click **Write Sunshine configuration and Desktop app**.
5. On one PC, open **Devices**. The other PC appears under **Nearby** after a
   few seconds. Click **Pair** and approve the request on the other PC.
6. Start **Mirror**, **Extend** or **Headless** from the peer card or the
   **Session** page.

The first session also pairs Moonlight with Sunshine automatically using a
one-time PIN. Later sessions connect directly.

## Modes and audio

| Mode | What happens | Default audio |
| --- | --- | --- |
| Mirror | Virtual output mirrors the physical output. | On |
| Extend | Virtual output is placed at the chosen edge at client-native resolution. | Off |
| Headless | Physical outputs are disabled; only the virtual output remains. | On |

Audio is a single on/off toggle per session. When off, Moonlight is launched
without an audio stream and Sunshine writes `stream_audio = disabled`. When on,
system audio is streamed. The host keeps its local audio in v1; muting the host
speakers via a PipeWire null sink is planned for v1.1.

The defaults can be changed in **Settings → Audio** or in `config.toml`:

```toml
[audio]
default_on_mirror = true
default_on_extend = false
default_on_headless = true
```

## CLI

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

## Documentation

- `docs/SETUP.md` - full installation and setup guide (dependencies, Sunshine,
  VA-API, audio, firewall, pairing, sessions, CLI)
- `docs/TROUBLESHOOTING.md` - symptom-based troubleshooting
- `docs/PROTOCOL.md` - control channel protocol
- `packaging/README.md` - building packages per distribution

The daemon writes `$XDG_CONFIG_HOME/kdefusion/config.toml` (normally
`~/.config/kdefusion/config.toml`). The file is created with `0600` permissions
and contains no secrets.

Key settings:

```toml
[device]
name = "Living Room PC"

[display]
virtual_name = "KDEFusion"      # KWin output: Virtual-KDEFusion
physical_output = ""            # empty = highest-priority enabled output
extend_position = "right"       # right | left | top | bottom
restore_on_stop = true

[streaming]
fps = 60
bitrate_kbps = 40000
codec = "auto"                  # auto | H.264 | HEVC | AV1
resolution = ""                 # empty = viewer native (extend/headless)

[control]
enabled = true
port = 47811

[api]
port = 47812

[sunshine]
manage = true
host = "127.0.0.1"
port = 47990
username = "kdefusion"
capture = "kwin"
```

Restart `kdefusiond` after changing ports, bind addresses or the device name.
Everything else can be changed live from the GUI or with `kdefusionctl set`.

State lives in `$XDG_STATE_HOME/kdefusion/` (normally
`~/.local/state/kdefusion/`): `state.json`, display snapshots and the PID lock.
Peer tokens and the Sunshine password live in the Secret Service; if
`secret-tool` is unavailable the daemon falls back to
`~/.local/state/kdefusion/secrets.json` with `0600`.

## How a session runs

Host side:

1. Snapshot the KScreen layout to `display-<session>.json`.
2. Start `krfb-virtualmonitor` with the mirror geometry or the client's native
   resolution, and wait for `Virtual-KDEFusion` to appear in KScreen.
3. Apply `kscreen-doctor` mirror/extend/headless options atomically and verify
   the result.
4. Write the Sunshine configuration and start Sunshine (or reuse the active
   user service).
5. Pair Moonlight with Sunshine automatically on first use.
6. Ask the viewer's daemon to launch Moonlight with the negotiated resolution,
   FPS, bitrate, codec and audio setting.
7. Monitor the viewer; stop Sunshine, remove the virtual output and restore the
   KScreen snapshot when the viewer disconnects or the user stops the session.

Viewer side:

1. Receive `start_session` on the authenticated control channel.
2. Launch `moonlight stream <host> Desktop ...` fullscreen.
3. Stop Moonlight when the host asks or when the session ends.

## Troubleshooting

See [docs/TROUBLESHOOTING.md](docs/TROUBLESHOOTING.md). Quick checks:

```bash
systemctl --user status kdefusiond
journalctl --user -u kdefusiond -n 200 --no-pager
kdefusionctl doctor
kscreen-doctor -o
vainfo | grep -E 'EncSlice|EncSliceLP'
```

Common causes:

- **No virtual output**: `krfb-virtualmonitor` is missing or KWin rejected the
  screencast. Check Diagnostics → KWin screencast permission.
- **Sunshine API 401**: Sunshine already has admin credentials. Save them in
  Settings → Sunshine.
- **Moonlight says the host is offline**: Sunshine is not running, port 47989
  (TCP) or the control port is blocked by the firewall.
- **Black stream**: capture is not `kwin`, or `output_name` does not match
  `Virtual-KDEFusion`.
- **Audio missing**: `pipewire-pulse` is not installed or the session default
  sink is unset.

## Security model (v1)

- The control channel is LAN-only by design. It binds `0.0.0.0:47811`, uses
  newline-delimited JSON, and requires a peer token for every action except the
  initial hello and pairing poll.
- Pairing requires explicit approval on both PCs and a six-digit code. Tokens
  are 256-bit random values stored in the Secret Service.
- The local API binds loopback only and has no authentication: any process
  running as your user could already control your session. Do not expose it to
  the network.
- The control protocol never executes shell commands. Only a fixed set of
  messages is accepted.
- Internet/WAN streaming is out of scope for v1. Do not port-forward the control
  port.

## Project layout

```
src/                 Rust daemon, CLI logic, Sunshine/KScreen/KWin helpers
src/installer.rs     package detection and Polkit package installer
src/bin/kdefusiond   daemon binary
src/bin/kdefusionctl CLI binary
src/bin/kdefusion    QML launcher binary
qml/Main.qml         Kirigami application window and navigation
qml/pages/           Devices, Session, Settings, Diagnostics
qml/components/      Backend HTTP client and peer card
packaging/           PKGBUILD, desktop file, icons
systemd/             user unit
scripts/             local install and packaging helpers
docs/                protocol and troubleshooting notes
```

## Development

```bash
cargo check --all-targets
cargo test
cargo run --bin kdefusiond -- --log-level debug --no-discovery
cargo run --bin kdefusion -- --smoke-test   # loads the QML and exits
```

The QML smoke test uses the offscreen platform automatically. To test the UI
against a running daemon:

```bash
CARGO_BUILD_JOBS=2 cargo build
./target/debug/kdefusiond --no-discovery --log-level debug &
KDEFUSION_QML_DIR=$PWD/qml QT_QPA_PLATFORM=wayland \
  /usr/lib/qt6/bin/qml qml/Main.qml
```

## Known limitations (v1)

- LAN only; no relay, NAT traversal or encryption beyond bearer tokens.
- Host speakers are not muted while streaming.
- The virtual output always comes from `krfb-virtualmonitor`; a natively
  headless KWin output is not used yet.
- One session per PC at a time.
- The GUI talks to the daemon over plain loopback HTTP.
- Automatic pairing currently assumes Sunshine is on the same PC as the host
  daemon and uses the control connection's source address.

## License

GPL-3.0-or-later. See `LICENSE`.
