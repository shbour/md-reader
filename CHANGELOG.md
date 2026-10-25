# Changelog

## 0.3.0 — 2026-10-25

### Added

- **Open a folder** with **Open Folder…** in the menu, the folder button next to
  **Open**, **Ctrl+Shift+O**, by dropping a folder on the window, or with
  `mdreader FOLDER` (`markdown-reader.exe FOLDER` on Windows). The sidebar gets a
  **Files** tab listing the folder's Markdown files as a tree. One click reads a
  file, so you can go through them one after another, and Back returns to the
  previous one. The **Contents** tab beside it still shows the document's headings.
- The open document is highlighted in the list, and the folders it is in are
  opened. The sidebar has **Refresh** and **Close Folder** buttons, and the
  folder is reopened at the next start.
- Windows: middle-click a file to read it in a new tab; the arrow keys move
  through the list and open or close folders.

### Details

- Hidden files and folders, `node_modules`, and folders with no Markdown in
  them are left out. Symlinked folders are not entered.
- Big folders (your home folder, say) are scanned in the background, one level
  at a time, and stop after 5,000 folders or files; the sidebar says when some
  files are not shown.
- Linux: with unsaved changes in the window, a file picked from the list opens
  in a new window, which shows the folder too. On Windows you are asked to save
  first, as when following a link.

## 0.2.0 — 2026-09-29

The first packaged release: the GNOME app for Linux and, new, a Windows app,
both built on the same rendering core.

- **Reading:** GitHub-flavoured Markdown (tables, task lists, footnotes,
  alerts), sanitised HTML, math with KaTeX, diagrams with Mermaid, syntax
  highlighting, a contents sidebar, and links between documents with Back.
- **Editing:** a side-by-side editor with live preview and line-accurate
  scroll sync, search, and following changes other programs make to the file.
- **Output:** print or save as PDF, and export a self-contained HTML page.
- **Windows app:** tabs that come back at the next start, per-tab undo history
  and scroll position, drag and drop of several files, `.md` file association,
  and dark mode. Windows (CRLF) line endings are kept.
- **Linux packages:** RPM for Fedora 40+ and RHEL 10 (with EPEL), and a .deb
  for Ubuntu 24.04+ and Debian 13+ that includes an AppArmor profile, without
  which WebKitGTK's sandbox can't start on Ubuntu 23.10 and later.
