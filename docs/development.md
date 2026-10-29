# Development

## Requirements

- macOS on Apple silicon or Intel
- [Rust](https://rustup.rs) 1.95 or newer
- [Bun](https://bun.sh)
- `cmake` (`brew install cmake`), used to build the bundled Opus decoder
- Python 3.10+ and Node.js, Deno or Bun, used at runtime to resolve streams

## Run

```sh
bun install
bun run tauri dev                    # the app, with hot reload
bun run dev                          # the UI alone in a browser, on mock data
```

`bun run dev` serves the UI at <http://localhost:1420> with a mock backend, so
you can work on the design without a YouTube session. Add `?signedin` to the
URL to start with a signed-in mock library.

## Build

```sh
bun run tauri build --bundles app    # target/release/bundle/macos/Encore.app
```

## Test

```sh
cargo test --workspace               # also regenerates src/lib/backend/bindings.ts
bun run check                        # svelte-check
```

Parser tests run against recorded responses in `crates/encore-ytm/fixtures/`.
Personal fields are scrubbed when they are recorded; never commit a recording
made with your own session without running it through `encore-cli record`.

## The command-line tool

`encore-cli` drives the backend without the app. It is the quickest way to
check a change against live YouTube Music.

```sh
cargo build -p encore-cli
C=target/debug/encore-cli

$C search "daft punk" --filter songs
$C album MPREb_7ltM34kr0mH
$C playlist VLPLgVLQgm4lkANkwLnIYZiwf6o2jKIc5Th3 --all
$C resolve 4D7u5KF7SP8               # sets up the yt-dlp environment on first run
$C play 4D7u5KF7SP8 --seek 60 --secs 20
$C lyrics 4D7u5KF7SP8

$C auth profiles                     # browsers and profiles Encore can import from
$C auth import-edge && $C auth select 0
$C whoami && $C liked && $C playlists
```

## Where Encore keeps things

| What | Where |
|---|---|
| Session | Keychain item `app.encore.desktop` / `youtube-music-session` |
| Library cache, settings, queue, lyrics cache | `~/Library/Application Support/app.encore.desktop/` |
| yt-dlp environment | same folder, `resolver/` |
| Audio (1 GB) and artwork (256 MB) caches | `~/Library/Caches/app.encore.desktop/` |
| Log | `~/Library/Logs/app.encore.desktop/encore.log` |

The log goes through a redacting writer. Cookies, tokens and signed stream
URLs must never reach it; `cargo test` includes a check for this.

## Known issues

- The first play of a song takes 5 to 8 seconds without an account, because
  YouTube delays stream links for signed-out listeners. The next song in the
  queue is prepared in the background to hide this.
- Started from Finder, Encore may not see your browser profiles. Give it Full
  Disk Access, or start it from a terminal.
- Each unsigned build asks once for access to the saved session in the
  Keychain.
- Switching audio output (for example to AirPods) causes a short gap.
