# Development

## Requirements

On every system:

- [Rust](https://rustup.rs) 1.95 or newer
- [Bun](https://bun.sh)
- `cmake`, used to build the bundled Opus decoder

Playback needs no Python, Node.js or Deno, on the system or downloaded: the
stream resolver and its JavaScript challenge solver are compiled into Encore
(see [The stream resolver](#the-stream-resolver)).

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

Add `?needsupdate` to see the notice Settings → Playback shows when YouTube
changed something only a newer Encore can follow (D37).

For updates (D24), an automatic check runs 3 seconds in: `?update` finds
Encore 0.5.0 and downloads it, `?update-manual` has automatic updates off,
`?update-deb` is a `.deb` install (a link instead of an update),
`?update-fail` makes checks fail, `?update-translocated` runs from a
translocated copy on macOS, and `?updated` starts just after an update,
with its "What's new".

For downloads, `?downloaded` starts with a playlist kept and downloaded and
one song on its own, `?offline` starts offline (switching the browser's
network off and on flips it too), and `?dlfail` makes every third download
fail as unavailable and the fifth meet a bot check, which pauses downloads
for 20 seconds without failing the song.

For the features after 0.4.0, the mock starts with a few bookmarks, saved
queues and folders (`?nosaved` for none) and five months of plays in History
→ Stats (`?nostats` for none). In your library, "Everything twice" has
duplicates and song/video pairs, and "The long way round" has 1,340 songs
that arrive a page at a time. Tidewalker's page shows a Wikipedia summary.

| Flag | Starts with |
|---|---|
| `?loop`, `?sleep` | an A–B loop on the loaded song; a sleep timer |
| `?autoplayoff` | Autoplay off (an artist's Shuffle plays their songs, not a radio) |
| `?blocked`, `?private` | a song and an artist blocked; a private session |
| `?lastfm`, `?lastfmexpired` | Last.fm connected; a session Last.fm refuses |
| `?discord`, `?discordoff`, `?nodiscord` | Discord on; on but not running; a build without a Discord app id |

## Build

```sh
bun run tauri build --bundles app           # macOS: bundle/macos/Encore.app
bun run tauri build --bundles nsis,msi      # Windows: bundle/nsis/*-setup.exe, bundle/msi/*.msi
bun run tauri build --bundles deb,appimage  # Linux: bundle/deb/*.deb, bundle/appimage/*.AppImage
```

Bundles land in `target/release/bundle/`. CI builds all three on every pull
request.

Discord presence needs a Discord application id, set as
`ENCORE_DISCORD_APP_ID` when building; without it the Discord row is hidden
and the feature is left out.

## Test

```sh
cargo test --workspace               # also regenerates src/lib/backend/bindings.ts
bun run check                        # svelte-check
bun test src/lib                     # the UI's pure logic (palette queries, find, sort, duplicates)
```

The resolver's solver corpus (`crates/encore-resolver/tests/ejs_players.rs`)
solves every player of yt-dlp-ejs's and yt-dlp's own tests, cold and from
the preprocessed player. The players are YouTube's code, so they are not in
the repository: the first run fetches them (about 160 MB) into
`$ENCORE_EJS_PLAYERS`, or Cargo's test tmp folder, and later runs are offline.
A player that can't be fetched (three tries) is skipped with a message, so a
CI runner YouTube won't answer doesn't fail the build; set
`ENCORE_EJS_STRICT=1` to make that a failure, as the weekly Solver corpus
workflow does and as you should locally before a release. A wrong answer
always fails.

Parser tests run against recorded responses in `crates/encore-ytm/fixtures/`.
Personal fields are scrubbed when they are recorded; never commit a recording
made with your own session without running it through `encore-cli record`.

To check that a real database migrates and comes back from its backups,
point the ignored tests at a **copy** (they change the file):

```sh
cp ~/Library/Application\ Support/app.encore.desktop/encore.sqlite* /tmp/db/
ENCORE_TEST_DB=/tmp/db/encore.sqlite cargo test -p encore-core real_database_migrates -- --ignored
ENCORE_TEST_DB=/tmp/db/encore.sqlite cargo test -p encore-core real_database_restores -- --ignored
```

Copy `encore.sqlite-wal` and `encore.sqlite-shm` with it if they exist, and
never point the tests at the original.

### Testing a built bundle

`encore --self-test [VIDEO_ID]` checks, without opening a window, what
depends on the machine: the data folder and database, the audio output,
the built-in challenge solver (offline, on a player written for the test;
always required) and, given a video id, resolving and decoding a real
stream. It prints a JSON report and exits
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
$C resolve 4D7u5KF7SP8 --diagnostics # every client asked, its answer, solve times
$C resolve 4D7u5KF7SP8 --client visionos   # one client only, no fallbacks
$C --auth session resolve 4D7u5KF7SP8      # as the signed-in channel (reads the Keychain)
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

$C downloads add 4D7u5KF7SP8         # download, and wait until it's done
$C downloads keep MPREb_7ltM34kr0mH  # keep an album downloaded (LM: liked songs)
$C downloads list                    # songs, kept collections, what waits
$C downloads sync                    # check kept collections for changes
$C downloads remove 4D7u5KF7SP8
$C downloads pause                   # the running app follows
$C downloads resume                  # and tries failed songs again
$C play 4D7u5KF7SP8 --offline        # from the downloads only, no YouTube
```

`resolve` runs signed out unless given `--auth session`; `play` and
`downloads` use the stored session unless given `--auth anonymous`, which
never reads the Keychain. `ENCORE_DATA_DIR` and `ENCORE_CACHE_DIR` point the
CLI's database, downloads, audio cache and the resolver's player scripts
somewhere else, so a test leaves the app's own alone (and a running
Encore's cache untouched):

```sh
ENCORE_DATA_DIR=/tmp/e ENCORE_CACHE_DIR=/tmp/e $C --auth anonymous downloads add 52fNscjkST4
```

Signed-in commands print outcomes, ping kinds and HTTP statuses only: never
cookies, tokens or tracking URLs.

## Where Encore keeps things

| What | macOS | Windows | Linux |
|---|---|---|---|
| Session | Keychain item `app.encore.desktop` / `youtube-music-session` | Credential Manager entry `youtube-music-session-key.app.encore.desktop` holds the key; the sealed session is `session.enc` in the data folder | Secret Service item, service `app.encore.desktop` |
| Last.fm key, secret and session (if connected) | Keychain item `app.encore.desktop` / `lastfm-session` | Credential Manager entry `lastfm-session.app.encore.desktop` | Secret Service item, service `app.encore.desktop`, account `lastfm-session` |
| Library cache, settings, queue, lyrics cache, your bookmarks, folders and play log | `~/Library/Application Support/app.encore.desktop/` | `%LOCALAPPDATA%\app.encore.desktop\` | `~/.local/share/app.encore.desktop/` |
| Backups of the database (the newest three, daily and before an update changes it) | `~/Library/Application Support/app.encore.desktop/backups/` | `%LOCALAPPDATA%\app.encore.desktop\backups\` | `~/.local/share/app.encore.desktop/backups/` |
| Folder covers you picked | `~/Library/Application Support/app.encore.desktop/covers/` | `%LOCALAPPDATA%\app.encore.desktop\covers\` | `~/.local/share/app.encore.desktop/covers/` |
| Downloads (with their covers and a sidecar per song) | `~/Library/Application Support/app.encore.desktop/downloads/` | `%LOCALAPPDATA%\app.encore.desktop\downloads\` | `~/.local/share/app.encore.desktop/downloads/` |
| Audio (1 GB) and artwork (256 MB) caches, the player scripts the resolver solves against and their preprocessed copies (`player/`, three of each at most), a downloaded update (`updates/`) | `~/Library/Caches/app.encore.desktop/` | `%LOCALAPPDATA%\app.encore.desktop\` (`audio\`, `art\`, `player\`, `updates\`) | `~/.cache/app.encore.desktop/` |
| Log | `~/Library/Logs/app.encore.desktop/encore.log` | `%LOCALAPPDATA%\app.encore.desktop\logs\encore.log` | `~/.local/state/app.encore.desktop/encore.log` |

The log goes through a redacting writer. Cookies, tokens and signed stream
URLs must never reach it; `cargo test` includes a check for this.

## The stream resolver

Encore finds a song's audio itself (D37 in [decisions.md](decisions.md)):
yt-dlp's YouTube extractor ported to Rust in `crates/encore-resolver`, and
yt-dlp-ejs, vendored unchanged, solving YouTube's JavaScript challenges on
QuickJS-ng compiled into the app. What it was ported from is pinned in
`crates/encore-resolver/upstream.toml`; [upstream.md](upstream.md) is how to
follow yt-dlp when YouTube changes something (`scripts/sync-upstream.sh`).
Encore 0.5's downloaded Python, Deno and yt-dlp are removed once, at the
first launch of 0.6.

## Updates

Encore updates itself from GitHub releases (D24). The publish checklist,
how to pull a release and who holds the signing key are in
[releases.md](releases.md).

- **Signing.** Release builds sign each updater package with a minisign
  key: `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
  are Actions secrets, given to the build step only. The public key is
  `plugins.updater.pubkey` in `tauri.conf.json`. `tauri.conf.json` has
  `bundle.createUpdaterArtifacts` off, so `bun run tauri build`, CI and
  forks need no key; the release workflow turns it on with
  `--config '{"bundle":{"createUpdaterArtifacts":true}}'`. To build signed
  packages locally, set both variables and pass the same option.
- **The manifest.** `scripts/release-manifest.sh <tag> <dist>` writes
  `latest.json` from the release's files and refuses on a missing or
  doubled platform, a version that differs from the tag, or a signature
  that doesn't verify (it needs `jq` and `minisign`).
  `scripts/test-release-manifest.sh` tests it with a throwaway key.
- **Testing an update.** Put one https URL in `update-endpoint` in the data
  folder and Encore checks that manifest instead of GitHub's (http is
  refused; the log says which endpoint it uses). To try a real update,
  build two versions with signed packages (`--config
  '{"version":"0.4.90",…}'`), serve the newer one's `.app.tar.gz` and a
  `latest.json` pointing at it over https (`tailscale serve` gives a
  trusted certificate without changing the system's trust), and point the
  older one at it. Development builds never check.

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
- Kept playlists and albums are checked for new songs only while Encore is
  open, and only for the account that downloaded them.
- Downloading many songs in a row can still make YouTube ask for a bot
  check. Downloads then wait 10 minutes, then 30, then stop until you try
  again.
- Installs from the `.deb` don't update themselves; Encore links to the
  new release instead.

macOS:

- Started from Finder, Encore may not see your browser profiles. Give it Full
  Disk Access, or start it from a terminal.
- Each unsigned build asks once for access to the saved session in the
  Keychain, so each update does too.
- Encore updates itself only from a folder it can write to. Opened from the
  disk image or from Downloads (a translocated copy), it asks to be moved
  to Applications first.
- Switching audio output (for example to AirPods) causes a short gap.

Windows:

- Builds are not code-signed, so SmartScreen warns once and Smart App Control
  blocks them.
- Browser import works for Firefox only (see the README).
- Closing the window quits Encore; there is no tray icon yet.

Linux:

- Without a Secret Service, Encore can't keep a session and says so when you
  sign in.
- If the window goes blank or closes by itself at start, WebKit's DMA-BUF
  renderer may be failing on your graphics driver. The AppImage turns it off
  already; for the `.deb`, start Encore with
  `WEBKIT_DISABLE_DMABUF_RENDERER=1 encore`. If it still happens, add
  `WEBKIT_DISABLE_COMPOSITING_MODE=1` and tell us.
- Closing the window quits Encore.
