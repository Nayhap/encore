<p align="center">
  <img src="docs/assets/thumbnail.webp" alt="Encore: YouTube Music for the desktop" width="900">
</p>

<p align="center">
  A desktop app for YouTube Music, built with Rust and Tauri.<br>
  Synced lyrics, a real queue and native playback, in about 20 MB.
</p>

<p align="center">
  <a href="https://github.com/Nayhap/encore/releases/latest">Download</a> ·
  <a href="docs/development.md">Build from source</a> ·
  <a href="docs/architecture.md">How it works</a>
</p>

---

> [!NOTE]
> Encore is unofficial and not affiliated with YouTube or Google. It talks to
> YouTube Music's private web API, which can change without notice. When it
> does, parts of the app may stop working until Encore is updated.

## Features

- **Synced lyrics** from YouTube Music and LRCLIB, in the side panel or full
  screen (<kbd>F</kbd>).
- **A seek bar drawn from the lyrics.** Each sung line is a segment, repeated
  lines stand taller, so you can see the chorus coming and jump to it.
- **Lyrics and the queue side by side** while you browse. Drag to reorder.
- **Search and commands in one box** (<kbd>⌘K</kbd>, or <kbd>Ctrl+K</kbd> on
  Windows and Linux). Shift+Enter plays next.
- **Your library**: playlists, liked songs, albums, artists, subscriptions.
  Brand channels are supported.
- **Playlists you can manage**: create, rename, describe, make private,
  unlisted or public, reorder by dragging, and delete. "Add to playlist"
  warns about duplicates and can start a new playlist.
- **Your history**: plays count in your YouTube Music history and
  recommendations, as in the official app (you can turn this off), and a
  History page shows what you played, by day.
- **Song or video**: switch between a song and its music video (audio
  only) from the now playing panel or any track's menu.
- **Native playback** in Rust, with seek, a disk cache and the next song
  loaded ahead of time.
- **Shuffle, repeat and autoplay** that work the way you expect.
- **Media keys and the system's media controls**: Now Playing on macOS, the
  media flyout on Windows, MPRIS on Linux.
- **Runs on macOS, Windows and Linux.**
- **Colours that follow the cover** of the song that's playing, in light or
  dark.

<p align="center">
  <img src="docs/assets/screens/lyrics.webp" width="49%" alt="Full-screen lyrics">
  <img src="docs/assets/screens/light.webp" width="49%" alt="A playlist in the light theme">
</p>

## Install

<p align="center">
  <img src="docs/assets/screens/windows-home-dark.webp" width="49%" alt="Encore on Windows 11, dark theme">
  <img src="docs/assets/screens/linux-home-light.webp" width="49%" alt="Encore on Linux, light theme">
</p>

Download the latest build from
[Releases](https://github.com/Nayhap/encore/releases/latest), or
[build it yourself](docs/development.md).

- **macOS**: `Encore.app`. Apple silicon and Intel.
- **Windows 10 and 11**: the `-setup.exe` installer (or the `.msi`). It
  installs for your user and needs no admin rights. Builds are not code-signed
  yet: SmartScreen warns on first run (choose **More info**, then **Run
  anyway**), and if Smart App Control is on, Windows blocks them.
- **Linux**: the `.deb` for Debian and Ubuntu, or the `.AppImage`. Needs
  WebKitGTK 4.1, and GNOME Keyring or KWallet to stay signed in.

Playback needs **Python 3.10 or newer** and **Node.js, Deno or Bun**. Encore
sets up its own copy of [yt-dlp](https://github.com/yt-dlp/yt-dlp) on first
launch and uses it to get audio streams.

| System | Install the requirements |
|---|---|
| macOS | `brew install python node` |
| Windows | Python from [python.org](https://www.python.org/downloads/) and Node.js from [nodejs.org](https://nodejs.org), or `winget install Python.Python.3.13 OpenJS.NodeJS.LTS` |
| Debian, Ubuntu | `sudo apt install python3 python3-venv nodejs` |
| Fedora | `sudo dnf install python3 nodejs` |

## Signing in

Encore works without an account. To see your library, open **Settings**, then
either:

- **Sign in**: opens a private Google sign-in window inside Encore. Works on
  every system.
- **Import from browser**: copies the YouTube session from a browser you are
  signed in to.
  - macOS: Edge, Chrome, Brave, Firefox or Safari. For the Chromium browsers,
    macOS asks once for access to the browser's "Safe Storage" key; choose
    **Always Allow**.
  - Linux: Edge, Chrome, Brave, Chromium or Firefox. The Chromium browsers'
    key comes from GNOME Keyring or KWallet, which may ask you to unlock it.
  - Windows: Firefox. Current Edge and Chrome protect their cookies with
    app-bound encryption and lock them while running, so other apps can't read
    them. Use **Sign in** for those.

Then pick the channel to use. Your session is stored in the system's
credential store and nowhere else: the macOS Keychain, the Secret Service on
Linux, or Windows Credential Manager (which holds the key to a sealed session
file, since a session is larger than it allows). Encore has no servers and sends no
analytics.

## Keyboard

| Key | Action |
|---|---|
| <kbd>Space</kbd> | Play or pause |
| <kbd>←</kbd> <kbd>→</kbd> | Seek 5 s (with <kbd>⇧</kbd>, 15 s) |
| <kbd>⌘←</kbd> <kbd>⌘→</kbd> | Previous or next song |
| <kbd>[</kbd> <kbd>]</kbd> | Previous or next lyric line |
| <kbd>S</kbd> / <kbd>R</kbd> / <kbd>L</kbd> | Shuffle / repeat / like |
| <kbd>F</kbd> | Full-screen lyrics |
| <kbd>⌘K</kbd> | Search and commands |
| <kbd>⌘\\</kbd> | Show or hide the side panel |
| <kbd>?</kbd> | All shortcuts |

On Windows and Linux, use <kbd>Ctrl</kbd> where this table says <kbd>⌘</kbd>.

## Contributing

Bug reports and pull requests are welcome. See
[CONTRIBUTING.md](CONTRIBUTING.md) to get set up.

## Acknowledgements

Encore learned a lot from [Meiro](https://github.com/elianiva/meiro),
[Kopuz](https://github.com/Kopuz-org/kopuz) and
[Better Lyrics](https://github.com/better-lyrics/better-lyrics). Streams come
through [yt-dlp](https://github.com/yt-dlp/yt-dlp), and synced lyrics through
[LRCLIB](https://lrclib.net).

## License

[MIT](LICENSE)
