#!/bin/sh
# PK-06: run as root in a debian:stable container with the .deb in /pkg. Installs it,
# checks what it put down, that its onyx is apt's (PK-02), then removes it.
set -eu
cd /pkg
apt-get update -qq
apt-get install -y -qq ./onyx_*_amd64.deb > /dev/null
version=$(dpkg-query -W -f '${Version}' onyx)
[ "$(onyx --version)" = "onyx $version" ]
for file in /usr/share/bash-completion/completions/onyx /usr/share/zsh/site-functions/_onyx \
  /usr/share/fish/vendor_completions.d/onyx.fish /usr/share/doc/onyx/copyright; do
  [ -s "$file" ] || { echo "missing $file"; exit 1; }
done
useradd -m tester
if su tester -c 'onyx update --dry-run' > said 2>&1; then exit 1; fi
grep -q 'onyx package.*with apt-get' said || { cat said; exit 1; }
apt-get remove -y -qq onyx > /dev/null
[ ! -e /usr/bin/onyx ]
echo "onyx $version .deb: ok"
