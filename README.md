# Snippet Library

A small Mac app for keeping keyboard shortcuts, text snippets, scripts, colors, images and links one click away.

- Search everything with ⌘F, then ↑ ↓ and Return to copy
- Favorites grid of one-click copy buttons
- Categories and tags
- Shortcut packs for Illustrator, InDesign, Photoshop, Premiere Pro, After Effects, Lightroom Classic and Figma (Mac defaults)
- Library saved as a plain JSON file in iCloud Drive, with 30 days of daily backups
- Dock app with an optional menu bar icon (turn it off from the ⋯ menu)

## Install

Download the latest `.dmg` from the [Releases page](../../releases/latest). The release notes explain how to open an app that isn't notarized by Apple.

## Where your data lives

| What | Where |
|---|---|
| Library | `iCloud Drive/Snippet Library/snippets.json` (change it from ⋯ > Change library folder) |
| Daily backups | `Backups/` next to the library, newest 30 kept |
| App settings | `~/Library/Application Support/com.snippetlibrary.desktop/settings.json` |

If iCloud Drive is turned off, the library goes to `~/Documents/Snippet Library/` instead.

## Browser version

`src/index.html` also works on its own in a browser. In Chrome or Edge it can save to a JSON file; in Firefox and Safari it saves inside the browser, with backup files you download.

## How it's built

- `src/index.html`: the whole interface (plain HTML, CSS and JavaScript, no framework)
- `src-tauri/`: the [Tauri 2](https://v2.tauri.app) shell in Rust (file saving, backups, clipboard, menu bar icon)
- `.github/workflows/build-macos.yml`: builds a universal app on every push to `main` and publishes it as the "latest" release. Pushing a tag like `v1.1.0` publishes a versioned release.

To build locally on a Mac: install the Xcode Command Line Tools (`xcode-select --install`), [Rust](https://rustup.rs), and the Tauri CLI (`cargo install tauri-cli --version "^2"`), then run `cargo tauri build` from the project folder.

## Icons

`src-tauri/icons/` holds plain placeholder icons. Replace them with your own (same file names and sizes) and push.
