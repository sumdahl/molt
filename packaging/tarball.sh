#!/bin/sh
# Packs a built onyx as the release tarball and its checksum (release.yml, ci.yml).
# Usage: packaging/tarball.sh <tag> <target>
set -eu
tag=$1 target=$2
name="onyx-$tag-$target"
mkdir "$name"
cp "target/$target/release/onyx" README.md CHANGELOG.md LICENSE "$name/"
tar czf "$name.tar.gz" "$name"
sha256sum "$name.tar.gz" > "$name.tar.gz.sha256"
