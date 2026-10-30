#!/bin/sh
# PK-04: writes the PKGBUILD for a release into <out>, with its version and the
# tarball's sha256. Usage: packaging/aur/fill.sh <version> <tarball> <out>
set -eu
version=$1 tarball=$2 out=$3
sum=$(sha256sum "$tarball" | cut -d' ' -f1)
mkdir -p "$out"
sed -e "s/^pkgver=.*/pkgver=$version/" -e "s/^sha256sums=.*/sha256sums=('$sum')/" \
  "$(dirname "$0")/PKGBUILD" > "$out/PKGBUILD"
cp "$tarball" "$out/"
