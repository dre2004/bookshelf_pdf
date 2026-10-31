# bookshelf_pdf

[![CI](https://github.com/dre2004/bookshelf_pdf/actions/workflows/ci.yml/badge.svg)](https://github.com/dre2004/bookshelf_pdf/actions/workflows/ci.yml)
[![License: AGPL-3.0-or-later](https://img.shields.io/badge/License-AGPL%20v3-blue.svg)](LICENSE)

A cross-platform PDF reader built for books — Windows, macOS and Linux.

Rendering is powered by [PDFium](https://pdfium.googlesource.com/pdfium/) (the
PDF engine used by Chrome), wrapped by [`pdfium-render`](https://crates.io/crates/pdfium-render)
on the Rust side of a [Tauri 2](https://v2.tauri.app) app, with a React +
TypeScript frontend.

## Features

- High-fidelity PDF rendering via PDFium, with a byte-bounded render cache
- Paged mode (keyboard / on-screen page turns) and continuous scroll mode
- Fit-width / fit-page zoom plus discrete zoom steps
- Library shelf with cover thumbnails, reading progress and a
  continue-reading card (stored in SQLite, non-destructive — files are never
  moved or modified)
- Auto-resume: reading position (page + position within the page) is saved
  continuously and restored when reopening a book
- Persistent settings: reading mode, theme and zoom survive restarts
- Password-protected PDFs (unlock prompt)
- Day / sepia / night themes
- (Roadmap: named bookmarks, full-text search, text selection & copy, outline
  and thumbnails, highlights & notes)

## Prerequisites

- [Node.js](https://nodejs.org) 20+
- [Rust](https://www.rust-lang.org/tools/install) (stable)
- Platform build requirements per the
  [Tauri 2 guide](https://v2.tauri.app/start/prerequisites/):
  - **Windows**: Visual Studio 2022 with the "Desktop development with C++"
    workload (MSVC toolchain **and** Windows SDK)
  - **macOS**: Xcode Command Line Tools
  - **Linux**: webkit2gtk and base dev libraries (see Tauri guide)

## Getting started

```bash
npm install
npm run fetch:pdfium   # downloads the PDFium library for your platform
npm run tauri dev
```

`npm run fetch:pdfium` downloads a prebuilt PDFium library from
[bblanchon/pdfium-binaries](https://github.com/bblanchon/pdfium-binaries) into
`src-tauri/resources/pdfium/` (git-ignored). For a different target pass its
name, e.g. `node scripts/fetch-pdfium.mjs linux-x64`. Set
`BOOKSHELF_PDFIUM_PATH` to point at a PDFium library in a custom location.

### Windows note

If your Windows SDK is installed on a non-default drive (or not registered
with Visual Studio), `vcvars` may not set `LIB`/`INCLUDE`/`PATH` correctly.
`scripts/win-env.cmd` wraps any build command with the full MSVC environment
plus SDK auto-detection, e.g.:

```bat
scripts\win-env.cmd npm run tauri dev
```

## Building a release bundle

Locally:

```bash
npm run tauri build
```

Installers land in `src-tauri/target/release/bundle/` (MSI/NSIS on Windows,
dmg on macOS, deb/rpm/AppImage on Linux). The PDFium library is bundled as a
resource automatically.

### Release pipeline

Push a version tag (`v0.1.0`, `v0.2.0`, …) and GitHub Actions builds every
platform and attaches the installers to a **draft** GitHub Release — review it
and publish. macOS builds are universal (one DMG for Intel and Apple Silicon,
unsigned).

```bash
git tag v0.1.0
git push origin v0.1.0
```

## Development

- `npm test` — frontend unit tests (vitest)
- `cargo test` in `src-tauri` — Rust unit tests plus a PDFium render smoke
  test (run `npm run fetch:pdfium` first)
- CI runs on every push and pull request: `cargo fmt --check`, strict clippy,
  Rust tests, frontend tests and a full typechecked build
  ([workflows](.github/workflows/))

## Architecture

- **Rendering** — PDFium renders pages to PNG bitmaps on the Rust side,
  served to the webview through a custom `pdfimg://` URI protocol. The
  browser handles image decoding/caching; no image bytes cross the IPC
  bridge. A byte-bounded LRU cache keys renders by (document, page, width).
- **Documents** — open documents live in a registry keyed by id; page
  dimensions are returned eagerly for the first 200 pages and fetched lazily
  in batches so thousand-page books open instantly.
- **Library & settings** — SQLite (`bookshelf.db` in the OS app-data dir)
  stores library entries, cover thumbnails, reading progress (page +
  position within the page) and a settings key-value store. Books are
  referenced by their original path; nothing is copied or modified.
- **Passwords** — password-required and wrong-password cases surface to the
  UI as distinct error codes from the `open_document` command.
- **Tests** — `cargo test` runs a render smoke test that exercises the real
  PDFium library (it creates a blank A4 document and renders it to PNG).

## License

Copyright © 2026 dre2004.

This project is licensed under the
[GNU AGPL-3.0-or-later](https://www.gnu.org/licenses/agpl-3.0.en.html) — the
strongest mainstream copyleft license. If you modify and distribute it (or run
a modified version as a network service), you must share your source under the
same license. PDFium itself is MIT/BSD-licensed
(`src-tauri/resources/pdfium/LICENSE.pdfium.txt` after fetching); see also the
third-party licenses of Tauri, pdfium-render and their dependencies.
