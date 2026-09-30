#!/bin/bash
# Build the Fedora / RHEL package of the GTK app from the committed tree.
#   packaging/build-rpm.sh [--nodeps]   (--nodeps: cargo from rustup, not dnf)
# Writes dist/markdown-reader-<version>-1.<dist>.x86_64.rpm
set -euo pipefail
cd "$(dirname "$0")/.."
[[ -f ~/.cargo/env ]] && . ~/.cargo/env
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
top=$(mktemp -d)
trap 'rm -rf "$top"' EXIT
mkdir -p "$top/SOURCES" dist
git archive --format=tar.gz --prefix="md-reader-$version/" -o "$top/SOURCES/md-reader-$version.tar.gz" HEAD
rpmbuild -bb "$@" --define "_topdir $top" --define "mdr_version $version" packaging/markdown-reader.spec
cp "$top"/RPMS/*/markdown-reader-*.rpm dist/
ls -1 dist/*.rpm
