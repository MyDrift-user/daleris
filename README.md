<div align="center">

<img src="assets/logo.png" width="128" alt="Bolt Explorer logo">

# Bolt Explorer

**A file explorer built for speed, with everything you expect from a modern one.**

Native, written in Rust, with no web view and no runtime to install.

[![CI](https://github.com/MyDrift-user/bolt-explorer/actions/workflows/ci.yml/badge.svg)](https://github.com/MyDrift-user/bolt-explorer/actions/workflows/ci.yml)
![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20Linux%20%7C%20macOS-4CA3FF)
![Rust](https://img.shields.io/badge/rust-2021-CE422B?logo=rust&logoColor=white)
![UI](https://img.shields.io/badge/UI-egui-4C6EF5)

<img src="assets/screenshot.png" width="800" alt="Bolt Explorer showing a folder with the sidebar and preview pane">

</div>

---

## ✨ Features

### ⚡ Fast search
A parallel crawler builds a compact in-memory index of the folder. The first search streams matches while it crawls, and every later keystroke searches the finished index on all CPU cores.

| Query | Finds |
|---|---|
| `report` | names containing "report" |
| `"final report"` | that exact text |
| `*.log` or `img_??` | wildcard matches |
| `-draft` | leaves out names containing "draft" |
| `ext:pdf` | files of one type |
| `size:>100mb` or `size:1g..5g` | files by size |
| `modified:today` or `date:>2026-01-01` | files by date |
| `hidden:yes` | hidden items |
| `/^img_\d+/` or `re:^img_\d+` | regular expressions |

### 🗂️ Everyday file management
- Tabs, with back, forward and up navigation and an editable address bar
- List and large icon views, with sorting and resizable columns
- Folder sizes counted in the background
- Preview pane for text files and folder summaries, plus a details panel for files
- Cut, copy, paste, rename and delete, including drag and drop

### ☁️ Cloud and network drives
- OneDrive, Google Drive, Dropbox and iCloud Drive folders appear in the sidebar
- On Windows, **Always keep on this device** and **Free up space** work directly from the menu
- Files that are only in the cloud aren't downloaded just to show a preview
- Map network drives on Windows, or connect to SMB and SFTP servers on Linux and macOS

### 🖱️ Native Windows integration
The real Windows icons, thumbnails and right-click menu, so your shell extensions keep working. You can also add your own commands to the menu, using `{path}`, `{paths}`, `{name}` and `{dir}`.

### 🎨 Make it yours
- Dark, light or follow Windows, with a choice of accent colours
- Icon packs, including your own made from PNG files
- A sidebar and ribbon bar you can rearrange
- A custom title bar, or the standard one

### 🏢 Ready for IT departments
On Windows, you can manage settings through **Group Policy (ADMX/ADML in English and German)** or Group Policy Preferences, as fixed settings or defaults. Open windows pick up changes within 30 seconds. See [`policy/README.txt`](policy/README.txt).

## ⌨️ Keyboard shortcuts

`Ctrl` is `Cmd` on macOS.

| Keys | Action |
|---|---|
| `Ctrl+T` / `Ctrl+W` | New tab / close tab |
| `Ctrl+Tab` | Next tab |
| `Ctrl+1`…`Ctrl+9` | Jump to a tab (`9` is the last one) |
| `Ctrl+F` or `F3` | Search |
| `Ctrl+L` or `Alt+D` | Edit the address |
| `Alt+←` / `Alt+→` / `Alt+↑` | Back / forward / up |
| `F5` | Refresh |
| `Ctrl+Shift+N` | New folder |
| `Alt+P` | Show or hide the preview pane |

---

## 📦 Install

Download the file for your system from **[Releases](../../releases)**, unpack it and run `bolt-explorer`. No installer is needed.

| System | File | Status |
|---|---|---|
| Windows 10 / 11 (x64) | `bolt-explorer-vX.Y.Z-windows-x64.zip` | Main platform |
| Linux (x64, X11 and Wayland) | `bolt-explorer-vX.Y.Z-linux-x64.tar.gz` | Builds, not yet tested in daily use |
| macOS (Apple Silicon) | `bolt-explorer-vX.Y.Z-macos-arm64.tar.gz` | Builds, not yet tested in daily use |

The macOS build isn't signed. If macOS blocks it, run `xattr -d com.apple.quarantine bolt-explorer`.

Bug reports for Linux and macOS are especially welcome.

## 🛠️ Build from source

You need [Rust](https://rustup.rs).

```sh
cargo build --release
```

The program ends up in `target/release/`.

On Debian or Ubuntu, install these libraries first:

```sh
sudo apt install libxkbcommon-dev libgl1-mesa-dev libwayland-dev libx11-dev
```

## 🚀 Releasing

Every push is built on all three systems by the CI workflow. Releases only start by hand:

1. Bump `version` in `Cargo.toml` and push.
2. Go to **Actions → Release → Run workflow** and enter the same version (e.g. `0.2.0`).

The workflow builds all three versions, tags `v<version>` and publishes a GitHub Release with the downloads.

## 🗺️ Project layout

```
src/
  main.rs                 entry point
  app.rs, app/            the interface: tabs, list, sidebar, menus, properties, settings
  search.rs               parallel crawler, index and query language
  dirsize.rs              background folder sizes
  fsx.rs, shell.rs        Windows file system and shell integration
  fsx_unix.rs, shell_unix.rs  the same for Linux and macOS
  policy.rs               Group Policy
  theme.rs                colours, icons and logo
policy/                   ADMX / ADML templates
```

---

<div align="center">
<sub>Made with 🦀 Rust and <a href="https://github.com/emilk/egui">egui</a>.</sub>
</div>
