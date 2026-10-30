#!/bin/sh
# PK-04: run as root in an archlinux container with the filled PKGBUILD and its
# tarball in /pkg. Builds the package as a normal user (the tarball is already there,
# so nothing is downloaded), writes .SRCINFO, installs it and checks what it put down.
set -eu
pacman -Syu --noconfirm --needed base-devel > /dev/null
useradd -m builder
chown -R builder /pkg
cd /pkg
su builder -c 'makepkg --noconfirm && makepkg --printsrcinfo > .SRCINFO'
pacman -U --noconfirm onyx-bin-*.pkg.tar.zst
version=$(sed -n 's/^pkgver=//p' PKGBUILD)
[ "$(onyx --version)" = "onyx $version" ]
for file in /usr/share/bash-completion/completions/onyx /usr/share/zsh/site-functions/_onyx \
  /usr/share/fish/vendor_completions.d/onyx.fish /usr/share/licenses/onyx-bin/LICENSE; do
  [ -s "$file" ] || { echo "missing $file"; exit 1; }
done
# PK-02: pacman's onyx is pacman's.
if su builder -c 'onyx update --dry-run' > said 2>&1; then exit 1; fi
grep -q 'onyx-bin package.*with pacman' said || { cat said; exit 1; }
pacman -R --noconfirm onyx-bin
[ ! -e /usr/bin/onyx ]
echo "onyx-bin $version: ok"
