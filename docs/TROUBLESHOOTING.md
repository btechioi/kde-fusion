# Troubleshooting

Run the built-in checks first:

```bash
systemctl --user status kdefusiond
journalctl --user -u kdefusiond -n 200 --no-pager
kdefusionctl doctor
```

The GUI's **Diagnostics** page shows the same checks with fixes.

## The daemon does not start

```bash
systemctl --user status kdefusiond
journalctl --user -u kdefusiond -n 100 --no-pager
```

- `another KDE Fusion daemon is already running`: stop the old process with
  `systemctl --user stop kdefusiond`, or kill the stale PID listed in
  `~/.local/state/kdefusion/daemon.pid`.
- Port already in use: change `[control] port` or `[api] port` in
  `~/.config/kdefusion/config.toml`, then restart the unit.
- The unit starts before the graphical session: the packaged unit is ordered
  after `graphical-session.target`. If you run it by hand from a TTY, KScreen,
  KWin and KWallet are not available.

## Installing Sunshine and Moonlight from the app

Open **Diagnostics**. The **Streaming packages** card lists Sunshine, Moonlight
and the KDE virtual monitor (`krfb`). Click **Install available packages**.

- The daemon detects the distribution and uses the native package manager for
  the packages that are available in the configured repositories.  On
  Arch/CachyOS this installs all three with `pacman`; on Fedora and Debian it
  installs `krfb` with `dnf`/`apt` and handles Sunshine and Moonlight through
  the vendor steps described below.
- Sunshine on Fedora is installed from LizardByte's COPR
  (`dnf copr enable lizardbyte/stable`, package `Sunshine`); on Debian install
  the official `.deb` from the Sunshine releases page.
- Moonlight on Fedora and Debian is installed as the Flatpak
  `com.moonlight_stream.Moonlight` from Flathub.  KDE Fusion launches it through
  `flatpak run`, so no native `moonlight` binary is required.  The AppImage can
  be used instead as long as it is installed as `moonlight` in `PATH`.
- The card shows the exact command and the live package-manager log. Keep the
  window open until it finishes; do not start a session during the transaction.
- If Polkit is not available, the card shows the command to run in a terminal.
- The installer only installs packages. Sunshine configuration, KWin screencast
  permission and the user service are prepared automatically when a session
  starts.

## No virtual output appears

`krfb-virtualmonitor` is a Wayland-only tool and creates `Virtual-<name>`.

```bash
command -v krfb-virtualmonitor
kscreen-doctor -o
journalctl --user -u kdefusiond -n 100 --no-pager | grep -i virtual
```

- Install `krfb`.
- Make sure the session type is Wayland:
  `echo $XDG_SESSION_TYPE` must print `wayland`.
- If a crash left `Virtual-KDEFusion` behind, the daemon kills the stale
  `krfb-virtualmonitor` on startup. You can also reboot or kill it manually.
- KWin may refuse to create more virtual outputs on very old KWin versions.
  Plasma 6.8+ is known to work.

## Sunshine cannot capture the output

Sunshine must use KWin capture and the exact virtual output name.

```bash
grep -E 'capture|output_name|encoder|stream_audio' ~/.config/sunshine/sunshine.conf
```

Expected:

```ini
capture = kwin
output_name = Virtual-KDEFusion
encoder = vaapi
```

KDE Fusion writes these keys when a session starts. If the output name differs,
clear `[display] virtual_name` back to `KDEFusion`.

KWin only grants `zkde_screencast_unstable_v1` to executables with a matching
desktop file. KDE Fusion creates
`~/.local/share/applications/dev.lizardbyte.app.Sunshine.kwin.desktop` when the
packaged one is missing. If capture still fails, log out and back in so KWin
reloads the permission file.

## Sunshine returns HTTP 401

Sunshine already has admin credentials and KDE Fusion's stored password is
wrong or missing.

1. Open the Sunshine web UI (`https://127.0.0.1:47990`) and sign in.
2. In KDE Fusion → Settings → Sunshine, enter the same username and password.
3. Click **Verify and save Sunshine credentials**.

If you do not know the password, stop Sunshine, back up and remove
`~/.config/sunshine/sunshine.conf`, start Sunshine again and set new
credentials. KDE Fusion will bootstrap a fresh install automatically.

## Moonlight pairing fails

The first session runs `moonlight pair` on the viewer while the host approves
the PIN through Sunshine's `/api/pin` endpoint. Failures usually mean:

- Moonlight reached a different PC (check the address shown in Diagnostics).
- Sunshine's API is not reachable on `127.0.0.1:47990` on the host.
- The viewer already paired, but `state.json` thinks it did not. Use
  **Devices → Remove**, then pair again. Alternatively start the session; the
  host treats "already paired" as success.

After a successful pairing, subsequent sessions connect without a PIN.

## Black screen or wrong size

```bash
kscreen-doctor -o
```

- Mirror should report `replication source:` pointing at the physical output.
- Extend must place `Virtual-KDEFusion` outside the physical geometry. The GUI
  uses the viewer's native pixel size at scale 1.
- Headless disables the physical outputs; KWin then has only the virtual one.
- If the stream is black, verify `output_name` and that Sunshine runs inside
  your Plasma session (user unit, not a system unit).
- If Moonlight shows the wrong resolution, set a resolution in
  **Settings → Streaming** or on the Session page's advanced options.

## Extend puts the virtual screen in the wrong place

The position is based on the physical output's *logical* size (physical pixels
divided by the KDE scale factor). For example a 1366×768 panel at scale 0.75 is
1822×1024 logical pixels. Change `[display] extend_position` or the physical
output selection; leave the scale at 1 for the virtual output.

## No audio

```bash
pactl info | grep 'Default Sink'
vainfo | grep -E 'EncSlice|EncSliceLP'
```

- Install `pipewire-pulse`.
- Make sure the session has a default output device.
- The session audio toggle must be on. Mirror and headless default to on;
  extend defaults to off.
- Restart the Sunshine user service after changing audio:
  `systemctl --user restart sunshine`.
- KDE Fusion does not mute the host speakers in v1; you will hear audio on both
  PCs if both are audible.

## Hardware encoding is unavailable

```bash
vainfo
```

Look for `VAEntrypointEncSlice` or `VAEntrypointEncSliceLP`:

- Intel: install `intel-media-driver`. Very old GPUs may only support H.264.
- AMD: install `libva-mesa-driver`.
- NVIDIA: Sunshine supports NVENC separately; KDE Fusion currently writes
  `encoder = vaapi` by default. Set `[sunshine] encoder` to the encoder name
  Sunshine expects for your system (for example `nvenc`) and run Diagnostics.
- If HEVC is unavailable, KDE Fusion automatically falls back to H.264.

## Display layout is not restored

KDE Fusion snapshots the layout before changing it and restores it on stop or on
the next daemon start after a crash. If restoration fails:

1. Open **System Settings → Display and Monitor**.
2. Re-enable the physical output and set the desired resolution/scale.
3. Check `~/.local/state/kdefusion/display-*.json` for the snapshot.

`restore_on_stop = false` disables automatic restoration.

## The GUI is blank or does not start

```bash
kdefusion --smoke-test
```

- The launcher looks for `/usr/lib/qt6/bin/qml`, then `qml6` on `PATH`.
  Override with `KDEFUSION_QML_RUNTIME=/path/to/qml`.
- It looks for `/usr/share/kdefusion/qml`, the install prefix next to the
  binary, and the repository `qml/` directory. Override with
  `KDEFUSION_QML_DIR=/path/to/qml`.
- Qt Quick Controls must use the KDE style. The launcher sets
  `QT_QUICK_CONTROLS_STYLE=org.kde.desktop`.
- The GUI talks to `http://127.0.0.1:47812`. If you changed the API port, either
  change it back or set `KDEFUSION_API`.

## The other PC is not discovered

- Both PCs must be on the same LAN and mDNS must not be blocked.
- Check `getent hosts <other-pc>.local` or browse with `avahi-browse -rt
  _kdefusion._tcp`.
- Firewalls must allow TCP 47811 and UDP 5353.
- mDNS is optional for control; you can add a peer by address through
  `kdefusionctl`/the API if discovery is unavailable.

## Reset pairing and state

```bash
systemctl --user stop kdefusiond
rm -f ~/.local/state/kdefusion/state.json
secret-tool clear service kdefusion key peer-token-<device-id>
systemctl --user start kdefusiond
```

Removing a peer from the GUI does the same without deleting all peers.

## Collecting a bug report

```bash
{
  echo "## status"; kdefusionctl status
  echo "## doctor"; kdefusionctl doctor
  echo "## kscreen"; kscreen-doctor -o
  echo "## vainfo"; vainfo
  echo "## unit"; systemctl --user status kdefusiond --no-pager
  echo "## log"; journalctl --user -u kdefusiond -n 300 --no-pager
} > kdefusion-report.txt
```

Remove tokens and personal paths before sharing.
