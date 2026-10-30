#!/bin/sh
# PK-05: builds onyx_<version>_amd64.deb and its checksum in <out> from the release
# tarball's binary. Usage: packaging/deb/build.sh <version> <tarball> <out>
set -eu
version=$1 tarball=$2 out=$3
work=$(mktemp -d)
tar xzf "$tarball" -C "$work"
from="$work/onyx-v$version-x86_64-unknown-linux-musl"
root="$work/root"
install -Dm755 "$from/onyx" "$root/usr/bin/onyx"
install -Dm644 "$from/LICENSE" "$root/usr/share/doc/onyx/copyright"
install -Dm644 "$from/README.md" "$root/usr/share/doc/onyx/README.md"
# PK-01: the completions come from the binary being packaged.
mkdir -p "$root/usr/share/bash-completion/completions" "$root/usr/share/zsh/site-functions" \
  "$root/usr/share/fish/vendor_completions.d"
"$from/onyx" completions bash > "$root/usr/share/bash-completion/completions/onyx"
"$from/onyx" completions zsh > "$root/usr/share/zsh/site-functions/_onyx"
"$from/onyx" completions fish > "$root/usr/share/fish/vendor_completions.d/onyx.fish"
mkdir "$root/DEBIAN"
cat > "$root/DEBIAN/control" <<CONTROL
Package: onyx
Version: $version
Architecture: amd64
Maintainer: Sumiran Dahal <50270034+sumdahl@users.noreply.github.com>
Section: utils
Priority: optional
Homepage: https://github.com/sumdahl/onyx
Description: Reclaim disk space on Linux, safely
 onyx finds caches, build folders and other regenerable data, shows a full
 dry run first, and removes only what you confirm.
CONTROL
mkdir -p "$out"
deb="onyx_${version}_amd64.deb"
dpkg-deb --root-owner-group --build "$root" "$out/$deb"
(cd "$out" && sha256sum "$deb" > "$deb.sha256")
rm -rf "$work"
