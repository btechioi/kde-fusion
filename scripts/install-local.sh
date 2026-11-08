#!/usr/bin/env bash
# Build and install KDE Fusion into a user prefix (default: ~/.local).
#
# This path needs no root and is the recommended way to try KDE Fusion on a
# development machine.  Use PREFIX=/usr with sudo only if you know what you are
# doing; the PKGBUILD is preferred for system-wide installs.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PREFIX="${PREFIX:-$HOME/.local}"
BINDIR="$PREFIX/bin"
DATADIR="$PREFIX/share"
ICONDIR="$DATADIR/icons/hicolor/scalable/apps"
UNITDIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
APPLICATIONSDIR="$DATADIR/applications"

if [[ ! -w "$(dirname "$PREFIX")" && ! -w "$PREFIX" ]]; then
  echo "error: $PREFIX is not writable. Re-run with sudo for PREFIX=/usr, or choose another PREFIX." >&2
  exit 1
fi

echo "==> Building release binaries"
cd "$ROOT"
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
cargo build --release --locked --jobs "${CARGO_BUILD_JOBS:-2}"

echo "==> Installing to $PREFIX"
install -d "$BINDIR" "$DATADIR/kdefusion" "$ICONDIR" "$APPLICATIONSDIR" "$UNITDIR"

# Install the Plasma widget (plasmoid) so it is available in the widget explorer.
PLASMOID_SRC="$ROOT/plasmoid"
if [[ -d "$PLASMOID_SRC" ]]; then
  PLASMOID_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/plasma/plasmoids/org.kde.plasma.kdefusion"
  rm -rf "$PLASMOID_DIR"
  mkdir -p "$PLASMOID_DIR"
  cp -a "$PLASMOID_SRC/." "$PLASMOID_DIR/"
fi
install -m755 target/release/kdefusiond "$BINDIR/kdefusiond"
install -m755 target/release/kdefusionctl "$BINDIR/kdefusionctl"
install -m755 target/release/kdefusion "$BINDIR/kdefusion"
# Copy the Qt QML runtime under a "kdefusion" name so Qt derives the
# application name (and thus the Wayland app id / taskbar icon) from it.
QML_RUNTIME="$(command -v qml-qt6 || command -v qml6 || command -v qml || true)"
if [[ -n "$QML_RUNTIME" ]]; then
  install -d "$PREFIX/lib/kdefusion"
  install -m755 "$QML_RUNTIME" "$PREFIX/lib/kdefusion/kdefusion"
fi

rm -rf "$DATADIR/kdefusion/qml"
cp -a "$ROOT/qml" "$DATADIR/kdefusion/qml"

install -m644 "$ROOT/packaging/kdefusion.svg" "$ICONDIR/kdefusion.svg"
install -m644 "$ROOT/packaging/kdefusion-symbolic.svg" "$ICONDIR/kdefusion-symbolic.svg"

sed "s|^Exec=kdefusion$|Exec=$BINDIR/kdefusion|" \
  "$ROOT/packaging/kdefusion.desktop" > "$APPLICATIONSDIR/kdefusion.desktop"
chmod 644 "$APPLICATIONSDIR/kdefusion.desktop"

sed "s|^ExecStart=/usr/bin/kdefusiond$|ExecStart=$BINDIR/kdefusiond|" \
  "$ROOT/systemd/kdefusiond.service" > "$UNITDIR/kdefusiond.service"
chmod 644 "$UNITDIR/kdefusiond.service"

# Refresh desktop and icon caches when the tools are available.  These are
# best-effort: a missing cache is not a fatal install error.
command -v update-desktop-database >/dev/null 2>&1 \
  && update-desktop-database "$APPLICATIONSDIR" >/dev/null 2>&1 || true
command -v gtk-update-icon-cache >/dev/null 2>&1 \
  && gtk-update-icon-cache -f -t "$DATADIR/icons/hicolor" >/dev/null 2>&1 || true
command -v systemctl >/dev/null 2>&1 \
  && systemctl --user daemon-reload >/dev/null 2>&1 || true
command -v systemctl >/dev/null 2>&1 \
  && systemctl --user enable --now kdefusiond >/dev/null 2>&1 || true

cat <<EOF

Installed KDE Fusion:
  daemon   $BINDIR/kdefusiond
  CLI      $BINDIR/kdefusionctl
  GUI      $BINDIR/kdefusion
  QML      $DATADIR/kdefusion/qml
  unit     $UNITDIR/kdefusiond.service

Next steps:
  1. Make sure $BINDIR is in PATH (log out and in, or export PATH="$BINDIR:\$PATH").
  2. Install runtime dependencies if they are missing (Arch: pacman, Fedora: dnf,
     Debian: apt).  The GUI's Diagnostics page lists the exact package names for
     your distribution and can install the streaming packages for you.
  3. Start the daemon and the GUI:
       systemctl --user enable --now kdefusiond
       kdefusion
  4. Open Diagnostics in the GUI and fix any errors.

To use KDE Fusion at login, keep the user unit enabled.  It starts with
graphical-session.target and needs your Plasma Wayland session.
EOF
