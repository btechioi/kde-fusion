#!/usr/bin/env bash
# Remove files installed by scripts/install-local.sh from a user prefix.
set -euo pipefail

PREFIX="${PREFIX:-$HOME/.local}"
BINDIR="$PREFIX/bin"
DATADIR="$PREFIX/share"
ICONDIR="$DATADIR/icons/hicolor/scalable/apps"
UNITDIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"

systemctl --user disable --now kdefusiond >/dev/null 2>&1 || true

rm -f "$BINDIR/kdefusiond" "$BINDIR/kdefusionctl" "$BINDIR/kdefusion"
rm -rf "$DATADIR/kdefusion"
rm -f "$ICONDIR/kdefusion.svg" "$ICONDIR/kdefusion-symbolic.svg"
rm -f "$DATADIR/applications/kdefusion.desktop"
rm -f "$UNITDIR/kdefusiond.service"

systemctl --user daemon-reload >/dev/null 2>&1 || true
echo "Removed KDE Fusion from $PREFIX."
echo "Configuration and state were kept in ${XDG_CONFIG_HOME:-$HOME/.config}/kdefusion and ${XDG_STATE_HOME:-$HOME/.local/state}/kdefusion."
