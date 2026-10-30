# Development

## Requirements

On every system:

- [Rust](https://rustup.rs) 1.95 or newer
- [Bun](https://bun.sh)
- `cmake`, used to build the bundled Opus decoder

Playback needs no Python or Node.js on the system: Encore downloads its own
(see [Player components](#player-components)).

### macOS

Apple silicon or Intel, with the Xcode Command Line Tools.

```sh
brew install cmake
```

### Windows

Windows 10 or 11 with the
[Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)
("Desktop development with C++") and the WebView2 runtime, which Windows 11
already has. Install Rust with the `x86_64-pc-windows-msvc` toolchain.

```powershell
winget install Kitware.CMake Oven-sh.Bun
```

Smart App Control blocks unsigned programs, and that includes Cargo's build
scripts, so it has to be off to build Encore.

### Linux

Debian or Ubuntu 24.04 and later (other distributions need the same
libraries):

```sh
sudo apt install build-essential cmake pkg-config libwebkit2gtk-4.1-dev \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev \
  libasound2-dev libdbus-1-dev patchelf
```

A Secret Service (GNOME Keyring or KWallet) is needed to stay signed in.

## Run

```sh
bun install
bun run tauri dev                    # the app, with hot reload
bun run dev                          # the UI alone in a browser, on mock data
```

`bun run dev` serves the UI at <http://localhost:1420> with a mock backend, so
you can work on the design without a YouTube session. Add `?signedin` to the
URL to start with a signed-in mock library, `?empty` to start with nothing
queued (a fresh install), and `?os=windows` or `?os=linux` to
see the window buttons and key labels of the other systems.

The mock starts without the player components, so the setup sheet shows
and plays a simulated install. Add `?ready` to start with them installed,
`?setupfail` to see the first install fail, or `?updating` to see a quiet
update.

## Build

```sh
bun run tauri build --bundles app           # macOS: bundle/macos/Encore.app
bun run tauri build --bundles nsis,msi      # Windows: bundle/nsis/*-setup.exe, bundle/msi/*.msi
bun run tauri build --bundles deb,appimage  # Linux: bundle/deb/*.deb, bundle/appimage/*.AppImage
```

Bundles land in `target/release/bundle/`. CI builds all three on every pull
request.

## Test

```sh
cargo test --workspace               # also regenerates src/lib/backend/bindings.ts
bun run check                        # svelte-check
```

Parser tests run against recorded responses in `crates/encore-ytm/fixtures/`.
Personal fields are scrubbed when they are recorded; never commit a recording
made with your own session without running it through `encore-cli record`.

### Testing a built bundle

`encore --self-test [VIDEO_ID]` checks, without opening a window, what
depends on the machine: the data folder and database, the audio output,
the player components (installed first if missing, as the setup screen
does) and, given a video id, resolving and decoding a real stream. It prints a JSON report and exits
non-zero if playback can't work. YouTube's bot checks don't count as a
failure unless you add `--strict`; on Windows add `--out report.json`, since
release builds have no console.

`scripts/self-test-bundle.sh` (`appimage`, `appimage-in <docker image>`,
`deb`, `app`) and `scripts/self-test-bundle.ps1` (the NSIS installer) run it
from the installed artifact, as users get it. CI runs them on every pull
request and the release workflow before it drafts a release.

## The command-line tool

`encore-cli` drives the backend without the app. It is the quickest way to
check a change against live YouTube Music.

```sh
cargo build -p encore-cli
C=target/debug/encore-cli

$C search "daft punk" --filter songs
$C album MPREb_7ltM34kr0mH
$C playlist VLPLgVLQgm4lkANkwLnIYZiwf6o2jKIc5Th3 --all
$C runtimes                          # player components: status, install, remove
$C resolve 4D7u5KF7SP8               # installs the components first if missing
$C play 4D7u5KF7SP8 --seek 60 --secs 20
$C lyrics 4D7u5KF7SP8

$C auth profiles                     # browsers and profiles Encore can import from
$C auth import-edge && $C auth select 0
$C whoami && $C liked && $C playlists

$C counterpart fa5IWHDbftI           # the song's music video, and the segment map if any
$C history                           # signed in: day groups, newest first
$C play 4D7u5KF7SP8 --secs 45 --report   # report the play, then read the history back
$C history remove 4D7u5KF7SP8        # remove that play again

$C playlist create "Encore test" --add 4D7u5KF7SP8   # private unless --privacy
$C playlist add PL… 4D7u5KF7SP8      # "duplicate"; add --anyway to add it again
$C playlist rename PL… "New name"
$C playlist privacy PL… unlisted
$C playlist move PL… 1 --before 0    # by row index
$C playlist delete PL…
```

Signed-in commands print outcomes, ping kinds and HTTP statuses only: never
cookies, tokens or tracking URLs.

## Where Encore keeps things

| What | macOS | Windows | Linux |
|---|---|---|---|
| Session | Keychain item `app.encore.desktop` / `youtube-music-session` | Credential Manager entry `youtube-music-session-key.app.encore.desktop` holds the key; the sealed session is `session.enc` in the data folder | Secret Service item, service `app.encore.desktop` |
| Library cache, settings, queue, lyrics cache, player components (`runtimes/`) | `~/Library/Application Support/app.encore.desktop/` | `%LOCALAPPDATA%\app.encore.desktop\` | `~/.local/share/app.encore.desktop/` |
| Audio (1 GB) and artwork (256 MB) caches | `~/Library/Caches/app.encore.desktop/` | `%LOCALAPPDATA%\app.encore.desktop\` (`audio\`, `art\`) | `~/.cache/app.encore.desktop/` |
| Log | `~/Library/Logs/app.encore.desktop/encore.log` | `%LOCALAPPDATA%\app.encore.desktop\logs\encore.log` | `~/.local/state/app.encore.desktop/encore.log` |

The log goes through a redacting writer. Cookies, tokens and signed stream
URLs must never reach it; `cargo test` includes a check for this.

## Player components

Playback runs yt-dlp on a Python and a Deno that Encore downloads itself
(D22 in [decisions.md](decisions.md)). The versions, URLs, sizes and SHA-256
checksums are pinned in `crates/encore-resolver/src/runtimes/manifest.rs`.
To move to newer ones, change all four together, taking each checksum from
the project's own release (python-build-standalone's `SHA256SUMS`, Deno's
`.sha256sum` files, PyPI's `sha256` digests), and run `cargo test`. Users who
already set up playback get the new versions on their next launch.

## Known issues

All systems:

- The first play of a song takes 5 to 8 seconds without an account, because
  YouTube delays stream links for signed-out listeners. The next song in the
  queue is prepared in the background to hide this.
- Plays are reported to YouTube history only when signed in. If your YouTube
  history is paused in your Google account, YouTube ignores them.
- Some libraries report that YouTube refuses "Remove from history" for brand
  channels. Encore shows when a removal was refused.
- Signed out, YouTube gives no timeline map between a song and its video, so
  switching versions starts the other one from the beginning. The video
  version plays the video's audio; there is no video picture.

macOS:

- Started from Finder, Encore may not see your browser profiles. Give it Full
  Disk Access, or start it from a terminal.
- Each unsigned build asks once for access to the saved session in the
  Keychain.
- Switching audio output (for example to AirPods) causes a short gap.

Windows:

- Builds are not code-signed, so SmartScreen warns once and Smart App Control
  blocks them.
- Browser import works for Firefox only (see the README).
- Closing the window quits Encore; there is no tray icon yet.

Linux:

- Without a Secret Service, Encore can't keep a session and says so when you
  sign in.
- Closing the window quits Encore.
