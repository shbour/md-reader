# Markdown Reader: the GTK 4 app (gtk/). Build with packaging/build-rpm.sh.
Name:           markdown-reader
Version:        %{mdr_version}
Release:        1%{?dist}
Summary:        Read, edit and preview Markdown files
License:        MIT
URL:            https://github.com/shbour/md-reader
Source0:        md-reader-%{version}.tar.gz

BuildRequires:  cargo >= 1.85
BuildRequires:  gcc
BuildRequires:  pkgconfig(gtk4) >= 4.14
BuildRequires:  pkgconfig(libadwaita-1) >= 1.5
BuildRequires:  pkgconfig(webkitgtk-6.0) >= 2.40
BuildRequires:  pkgconfig(gtksourceview-5)
BuildRequires:  desktop-file-utils
BuildRequires:  appstream

# Rust builds its own debug info; the release profile strips the binary.
%global debug_package %{nil}

%description
Markdown Reader opens Markdown files as clean, readable pages and lets you
edit them side by side with a live preview: GitHub-flavoured Markdown, math
(KaTeX), diagrams (Mermaid), a table of contents, search, print, and export
to PDF or a self-contained HTML page.

%prep
%autosetup -n md-reader-%{version}

%build
cargo build --release --locked -p md-reader

%install
install -Dm755 target/release/mdreader %{buildroot}%{_bindir}/mdreader
install -Dm644 data/io.github.mdreader.MdReader.desktop %{buildroot}%{_datadir}/applications/io.github.mdreader.MdReader.desktop
install -Dm644 data/io.github.mdreader.MdReader.svg %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/io.github.mdreader.MdReader.svg
install -Dm644 data/io.github.mdreader.MdReader.metainfo.xml %{buildroot}%{_metainfodir}/io.github.mdreader.MdReader.metainfo.xml

%check
desktop-file-validate %{buildroot}%{_datadir}/applications/io.github.mdreader.MdReader.desktop
appstreamcli validate --no-net %{buildroot}%{_metainfodir}/io.github.mdreader.MdReader.metainfo.xml

%files
%license LICENSE
%doc README.md
%{_bindir}/mdreader
%{_datadir}/applications/io.github.mdreader.MdReader.desktop
%{_datadir}/icons/hicolor/scalable/apps/io.github.mdreader.MdReader.svg
%{_metainfodir}/io.github.mdreader.MdReader.metainfo.xml

%changelog
* Tue Sep 29 2026 shbour <23043611+shbour@users.noreply.github.com> - 0.2.0-1
- First packaged release
