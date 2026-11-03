# Architecture

Encore is a Tauri app. The UI is Svelte 5 running in the system webview. Everything else
(talking to YouTube Music, sign-in, audio, lyrics, the queue) is Rust, in crates that do
not depend on Tauri, so they can be tested and run from the command line.

```
┌──────────────── src/ (Svelte) ────────────────┐
│ views, components, state.svelte.ts            │
│ src/lib/backend: typed bindings + mock        │
└──────────────┬────────────────────────────────┘
               │ commands ↓   events ↑   art:// images
┌──────────────┴──── src-tauri/ ────────────────┐
│ commands, events, art:// protocol,            │
│ Now Playing and media keys, sign-in window    │
└──────────────┬────────────────────────────────┘
               │
   features/ ── one module per area (saved, taste, stats, lastfm, …)
     ├─ encore-lastfm     Last.fm scrobbles and play counts
     ├─ encore-discord    Discord Rich Presence over local IPC
     ├─ encore-wiki       Wikipedia summaries for artist pages
     └─ encore-analysis   tempo and key estimates from audio files
   encore-downloads ── downloads index, fetching, kept collections
   encore-player ── queue, engine, resolver, radio, preload
     ├─ encore-ytm        YouTube Music API client and parsers
     ├─ encore-auth       cookies, browser import, Keychain
     ├─ encore-resolver   stream URLs via a managed yt-dlp + its runtimes
     ├─ encore-audio      decode and output
     ├─ encore-lyrics     lyrics sources, matching, cache
     └─ encore-core       shared types, queue model, storage
```

## Crates

| Crate | What it does |
|---|---|
| `encore-core` | Domain types, the `MusicService` and `StreamResolver` traits, the queue, the SQLite store with its backups and the local data of `local` (bookmarks, folders, the play log, ...), the paced background lane for long lists (`paging`), artwork keys, log redaction. |
| `encore-ytm` | YouTube Music's internal API (`WEB_REMIX`, plus `ANDROID_MUSIC` for lyrics). Typed parsers for its responses, continuations, likes, playlist management, history, song/video pairs, play reporting to history, account and channel listing. |
| `encore-auth` | The cookie jar, the `SAPISIDHASH` authorization header (including brand channels), cookie readers for Chromium browsers, Firefox and Safari, and the Keychain session store. |
| `encore-resolver` | Downloads, verifies and keeps the player components (Python, Deno, yt-dlp; D22), and runs the pinned yt-dlp on them as a long-lived helper. Cookies are passed per request in memory, never written to disk. Each URL is probed before playback. |
| `encore-audio` | Symphonia (AAC, and Opus through libopus) decoding to cpal, with a lock-free ring buffer and resampling. Streams over HTTP ranges into a disk cache. |
| `encore-player` | The controller. Owns the queue, drives the engine, resolves and preloads the next song, fetches radio for autoplay, and emits events. Its play observer writes every audible play to the local log. |
| `encore-lyrics` | YouTube Music timed lyrics, then LRCLIB synced, then plain lyrics. Scores LRCLIB matches by title, artist and duration, and caches results. |
| `encore-downloads` | Songs kept for offline listening (D23): one worker resolves through the yt-dlp helper and fetches checked HTTP ranges, one queue checks kept playlists, albums and liked songs for changes, and lyrics and covers are kept alongside. It depends on `encore-core`, `encore-audio` and `encore-lyrics`; the player reaches it only through `encore-core`'s `LocalAudio` trait and asks it for a song's file before resolving anything. |
| `encore-lastfm` | Last.fm with the user's own API key (D31): desktop auth, now playing, an offline scrobble queue, play counts for sorting. Every call is a signed POST. |
| `encore-discord` | Discord Rich Presence (D31) over Discord's local socket or named pipe, with its own frames; no SDK. |
| `encore-wiki` | An artist's Wikipedia summary (D29) from the REST API, strictly matched, cached 30 days. |
| `encore-analysis` | Tempo and key estimated from a downloaded or cached file (D28), on demand. |
| `encore-cli` | A headless tool that exercises every crate against live YouTube Music. |

## UI contract

Commands and events are typed end to end with
[tauri-specta](https://github.com/specta-rs/tauri-specta); the TypeScript
bindings are generated into `src/lib/backend/bindings.ts` by `cargo test`.

- **State is owned by Rust.** The queue, shuffle, repeat and playback state live in
  `encore-player`. The Svelte `Player` class mirrors them and turns user actions into
  commands.
- **Position is not streamed.** Rust sends a position anchor twice a second and
  immediately on seek or pause; the UI interpolates between anchors each frame.
- **Events carry revisions**, so a late event for an old track or an old seek is ignored.
- **Artwork** loads through an `art://` protocol that takes opaque keys, checks the host,
  and caches to disk. The cover's dominant hue is computed once and drives the UI accent.

## Feature areas

Features from 0.5 on live in areas with the same three seams on each side,
so they can grow apart: `src-tauri/src/features/<area>.rs` (commands,
events, state, an `install` run at startup), `src/lib/backend/features/<area>.ts`
and `backend/mock/<area>.ts` (the area's calls, under `api.features`), and
`src/lib/features/<area>.svelte.ts` (its UI state). Shared surfaces don't
list feature actions themselves: track, collection, artist and player bar
menus, the palette, the player bar's chips and single-key shortcuts ask the
lists in `src/lib/features/hooks.ts`, which each area adds to.

| Area | What it holds |
|---|---|
| `playback` | A–B loop, sleep timer, shuffle cycles, Autoplay (D26, D33) |
| `library_tools` | Find, sort, select, duplicates, discography (D34, D35) |
| `saved` | Bookmarks, saved queues, folders and their covers (D25, D33, D34) |
| `taste`, `stats` | Block list, Hide podcasts, private sessions; the play log's Stats (D27, D30, D36) |
| `lastfm`, `presence`, `wiki`, `details` | Last.fm, Discord, Wikipedia, song details (D28, D29, D31) |
| `ambience`, `power` (UI only) | Cover ambience, the Power Bar (D27, D32) |

Rust state an area shares with the player goes through the player, not
around it: the block list is the player's `Admission`, and a private session
is the player's flag and clock, which history, Last.fm and Discord follow.

## Sign-in

Sessions come from one of two places:

1. **Browser import.** Encore reads the selected browser profile's cookie database,
   decrypts it (Chromium browsers keep the key in the Keychain under "Safe Storage"),
   keeps only YouTube and Google cookies, and refreshes the session straight away.
2. **In-app sign-in.** A private (non-persistent) webview window on
   `accounts.google.com`. Encore waits until the sign-in is complete before reading its
   cookies, then closes it.

Either way, the user then picks a channel, which may be a brand channel. The session is
saved as one Keychain item and is kept alive by periodic refreshes. Nothing secret is
written to disk or to the log.

## Offline

Encore counts as offline only after a request got no answer and a probe of
music.youtube.com agrees; one probe that gets any answer brings it back. Offline,
the player plays downloaded songs and passes over the others with one rule
for next, previous, the end of a song and preloading. Radio, likes, playlist
edits and search wait, history pings are neither sent nor kept for later,
and lyrics and covers come from what was kept with the downloads.

## When YouTube changes things

Requests and parsers fail loudly and specifically. A song that cannot be played is
skipped with a visible reason, at most three times in a row. Stream resolution is behind
the `StreamResolver` trait, so yt-dlp can be updated or replaced without touching the
rest of the app.

The numbered design decisions that code comments refer to, such as `(D6)`, are in
[decisions.md](decisions.md).
