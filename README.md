<div align="center">

# Ts-Music

*A native music album player built with Tauri + Vue*

![Tauri](https://img.shields.io/badge/Tauri-24C8DB?style=for-the-badge&logo=Tauri&logoColor=white)
![Vue](https://img.shields.io/badge/Vue.js-35495E?style=for-the-badge&logo=vuedotjs&logoColor=4FC08D)
![Rust](https://img.shields.io/badge/Rust-%23000000.svg?style=for-the-badge&logo=rust&logoColor=white)
![Vite](https://img.shields.io/badge/Vite-646CFF?style=for-the-badge&logo=Vite&logoColor=white)
![JavaScript](https://img.shields.io/badge/JavaScript-F7DF1E?style=for-the-badge&logo=JavaScript&logoColor=000)
![Tailwind CSS](https://img.shields.io/badge/Tailwind_CSS-grey?style=for-the-badge&logo=tailwind-css&logoColor=38B2AC)

</div>

## Overview

Ts-Music is a lightweight, cross-platform desktop music player. It scans a local
folder for audio files, reads their metadata (title, artist, album, year, track
number, cover art), and organizes your library into browsable **Songs**,
**Albums**, and **Artists** views.

The heavy lifting — recursive directory traversal and audio tag parsing — runs in
a native Rust backend (Tauri), while the user interface is a responsive Vue 3
single-page application. This keeps scanning fast even for large libraries, with
an optional multi-threaded mode powered by [Rayon](https://github.com/rayon-rs/rayon)
and [jwalk](https://github.com/Byron/jwalk).

## Features

- **Local library scanning** — pick any folder and recursively scan it for music.
- **Rich metadata parsing** — reads title, artist, album, year, track number, and
  duration via [Lofty](https://github.com/Serial-ATA/lofty-rs).
- **Embedded cover art** — extracts album artwork directly from the audio files.
- **Three library views** — browse by Songs, Albums, or Artists, with dedicated
  album and artist detail pages.
- **Full playback controls** — play/pause, next/previous, seek, volume, shuffle,
  and three loop modes (off / loop all / loop one).
- **Instant search** — filter the library by title, artist, or album in real time.
- **Optional parallel scanning** — toggle multi-threaded scanning for faster
  imports on multi-core machines.
- **Persistent library** — the scanned library is cached in `localStorage`, so it
  loads instantly on the next launch.
- **Incremental imports** — adding a new folder merges new tracks without
  duplicating existing ones.

## Supported Audio Formats

`.mp3` &nbsp;·&nbsp; `.flac` &nbsp;·&nbsp; `.wav` &nbsp;·&nbsp; `.m4a` &nbsp;·&nbsp; `.ogg` &nbsp;·&nbsp; `.aac`

## Tech Stack

| Layer        | Technology                                                   |
| ------------ | ------------------------------------------------------------ |
| Desktop shell| [Tauri 2](https://tauri.app/)                                |
| Frontend     | [Vue 3](https://vuejs.org/) + [Vue Router](https://router.vuejs.org/) |
| Build tool   | [Vite 6](https://vitejs.dev/)                                |
| Styling      | [Tailwind CSS 3](https://tailwindcss.com/)                   |
| Backend      | [Rust](https://www.rust-lang.org/)                           |
| Audio tags   | [Lofty](https://github.com/Serial-ATA/lofty-rs)             |
| Concurrency  | [Rayon](https://github.com/rayon-rs/rayon) + [jwalk](https://github.com/Byron/jwalk) |

## Project Structure

```
ts-music-serial/
├── src/                      # Vue 3 frontend
│   ├── components/           # Reusable UI components (player, song list, etc.)
│   ├── views/                # Page-level views (Songs, Albums, Artists, Settings)
│   ├── router/               # Vue Router configuration
│   ├── store.js              # Reactive global state (library + player)
│   └── main.js               # App entry point
├── src-tauri/                # Rust backend
│   ├── src/lib.rs            # Tauri commands: scan_music_folder, get_track_cover
│   ├── Cargo.toml            # Rust dependencies
│   └── tauri.conf.json       # Tauri app configuration
├── public/                   # Static assets
├── index.html                # HTML entry point
└── package.json              # Frontend dependencies and scripts
```

## Prerequisites

- [Node.js](https://nodejs.org/) (v18 or newer recommended)
- [Rust](https://www.rust-lang.org/tools/install) (stable toolchain)
- Platform-specific Tauri dependencies — see the
  [Tauri prerequisites guide](https://tauri.app/start/prerequisites/).

## Getting Started

Clone the repository and install the frontend dependencies:

```bash
git clone https://github.com/iniFaiz/ts-music-serial.git
cd ts-music-serial
npm install
```

### Run in development

Launches the app with hot-reload for both the Vue frontend and the Rust backend:

```bash
npm run tauri dev
```

### Build for production

Produces a native installer/binary for your platform under
`src-tauri/target/release/`:

```bash
npm run tauri build
```

## Available Scripts

| Command                | Description                                          |
| ---------------------- | ---------------------------------------------------- |
| `npm run dev`          | Start the Vite dev server (frontend only).           |
| `npm run build`        | Build the frontend for production.                   |
| `npm run preview`      | Preview the production frontend build.               |
| `npm run tauri dev`    | Run the full desktop app in development mode.        |
| `npm run tauri build`  | Bundle the desktop app for distribution.             |

## How It Works

1. **Add a folder** — the frontend opens a native directory picker and passes the
   selected path to the Rust backend.
2. **Scan** — the `scan_music_folder` command walks the directory tree
   (sequentially with `walkdir`, or in parallel with `jwalk` + `rayon`), filters
   for supported audio files, and parses each file's tags with Lofty.
3. **Organize** — the returned tracks are sorted by artist → album → track number
   → title, cached in `localStorage`, and grouped into the Songs, Albums, and
   Artists views.
4. **Play** — selecting a track loads its bytes through the Tauri filesystem
   plugin and plays it via the HTML5 `<audio>` element, while cover art is fetched
   on demand through the `get_track_cover` command.

## Contributors

<div align="center">

<a href="https://github.com/iniFaiz"><img src="https://avatars.githubusercontent.com/u/64961002?v=4" title="Faiz Alrazi Hidayat" width="80" height="80"></a>
<a href="https://github.com/Bluenomic"><img src="https://avatars.githubusercontent.com/u/149468407?v=4" title="Imam Dzulvan Muffid" width="80" height="80"></a>
<a href="https://github.com/nfgcode"><img src="https://avatars.githubusercontent.com/u/50001308?v=4" title="Nurfauzan Gymnastiar" width="80" height="80"></a>
<a href="https://github.com/ReezqiAseli"><img src="https://avatars.githubusercontent.com/u/111623692?v=4" title="Mochammad Reezqi Pratama" width="80" height="80"></a>
<a href="https://github.com/kingpentes"><img src="https://avatars.githubusercontent.com/u/146577113?v=4" title="Hadri Harazit" width="80" height="80"></a>

</div>
