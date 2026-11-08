# Packaging KDE Fusion

## Arch Linux / CachyOS

```bash
./scripts/make-pkg-source.sh
cd packaging
makepkg -si
```

## Fedora / openSUSE

```bash
./scripts/make-pkg-source.sh
rpmbuild -tb kde-fusion-0.1.14.tar.gz
sudo dnf install ~/rpmbuild/RPMS/x86_64/kde-fusion-0.1.0-1.*.rpm
```

The spec file is `packaging/kde-fusion.spec`.

## Debian / Ubuntu

KDE Fusion can be installed from source without packaging:

```bash
./scripts/install-local.sh
```

For a .deb, copy `packaging/debian/` to the repository root as `debian/` and
build with `dpkg-buildpackage -us -uc`.  The provided files are a starting
point and may need adjustment for your release.