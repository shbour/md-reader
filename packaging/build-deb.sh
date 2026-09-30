#!/bin/bash
# Build the Debian / Ubuntu package of the GTK app. Run it on the oldest
# release to support (Ubuntu 24.04): the package then also installs on newer
# ones. Dependencies come from dpkg-shlibdeps, i.e. the libraries the binary
# links. Writes dist/markdown-reader_<version>_<arch>.deb
set -euo pipefail
umask 022
cd "$(dirname "$0")/.."
[[ -f ~/.cargo/env ]] && . ~/.cargo/env
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
arch=$(dpkg --print-architecture)
cargo build --release --locked -p md-reader

root=$(mktemp -d)
trap 'rm -rf "$root"' EXIT
chmod 755 "$root" # becomes / in the package
install -Dm755 target/release/mdreader "$root/usr/bin/mdreader"
install -Dm644 data/io.github.mdreader.MdReader.desktop "$root/usr/share/applications/io.github.mdreader.MdReader.desktop"
install -Dm644 data/io.github.mdreader.MdReader.svg "$root/usr/share/icons/hicolor/scalable/apps/io.github.mdreader.MdReader.svg"
install -Dm644 data/io.github.mdreader.MdReader.metainfo.xml "$root/usr/share/metainfo/io.github.mdreader.MdReader.metainfo.xml"
install -Dm644 README.md "$root/usr/share/doc/markdown-reader/README.md"
{
    echo "Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/"
    echo "Upstream-Name: md-reader"
    echo "Source: https://github.com/shbour/md-reader"
    echo
    echo "Files: *"
    echo "License: MIT"
    sed 's/^$/./; s/^/ /' LICENSE
} > "$root/usr/share/doc/markdown-reader/copyright"
{
    echo "markdown-reader ($version) unstable; urgency=medium"
    echo
    echo "  * Release $version: https://github.com/shbour/md-reader/releases/tag/v$version"
    echo
    echo " -- shbour <23043611+shbour@users.noreply.github.com>  $(date -R -d "$(git log -1 --format=%cI 2>/dev/null || date -R)")"
} | gzip -9n > "$root/usr/share/doc/markdown-reader/changelog.gz"

# dpkg-shlibdeps wants a debian/control to sit next to it.
work=$(mktemp -d)
mkdir -p "$work/debian"
printf 'Source: markdown-reader\n\nPackage: markdown-reader\nArchitecture: any\n' > "$work/debian/control"
depends=$(cd "$work" && dpkg-shlibdeps -O "$root/usr/bin/mdreader" | sed -n 's/^shlibs:Depends=//p')
rm -rf "$work"

mkdir -p "$root/DEBIAN"
cat > "$root/DEBIAN/control" <<CONTROL
Package: markdown-reader
Version: $version
Architecture: $arch
Maintainer: shbour <23043611+shbour@users.noreply.github.com>
Installed-Size: $(du -sk --exclude=DEBIAN "$root" | cut -f1)
Depends: $depends
Section: text
Priority: optional
Homepage: https://github.com/shbour/md-reader
Description: Read, edit and preview Markdown files
 Markdown Reader opens Markdown files as clean, readable pages and lets you
 edit them side by side with a live preview: GitHub-flavoured Markdown, math
 (KaTeX), diagrams (Mermaid), a table of contents, search, print, and export
 to PDF or a self-contained HTML page.
CONTROL

mkdir -p dist
out="dist/markdown-reader_${version}_${arch}.deb"
dpkg-deb --root-owner-group -Zxz --build "$root" "$out" >/dev/null
echo "$out"
