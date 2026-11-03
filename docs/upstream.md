# Following yt-dlp

Encore resolves songs with its own extractor (D37 in
[decisions.md](decisions.md)). It is yt-dlp's YouTube extractor ported to
Rust, module for module, plus yt-dlp-ejs, vendored unchanged, for the
JavaScript challenges. When YouTube changes something, yt-dlp usually has
the fix within days; this page is how to bring it over.

## What is pinned

`crates/encore-resolver/upstream.toml` holds every pin, and tests read it:

- `[yt_dlp]`: the commit the Rust modules were read at, and its version.
- `[ejs]`: the yt-dlp-ejs release yt-dlp pins (`jsc/_builtin/vendor/_info.py`),
  the SHA3-512 of `yt.solver.lib.min.js` and `yt.solver.core.min.js` copied
  from that file, and the ejs commit the solver fixtures were read at.
- `[engine]`: `rquickjs` (pinned exactly in `Cargo.toml`) and the QuickJS-ng
  it bundles.
- `[fixtures]`: the player ids of the solver corpus.

## Where things are

| yt-dlp (`yt_dlp/extractor/youtube/`) | Encore (`crates/encore-resolver/src/`) |
|---|---|
| `_base.py` `INNERTUBE_CLIENTS`, `build_innertube_clients`, PO token policies | `base/innertube_clients.json` (data, same keys, `_line` per entry), `base/clients.rs` |
| `_base.py` `generate_api_headers`, SAPISIDHASH, `_extract_context`, consent and `PREF` | `base/mod.rs` |
| `_base.py` `_download_ytcfg`, `extract_ytcfg` | `base/ytcfg.rs` |
| `_video.py` `_DEFAULT_*CLIENTS`, `_get_requested_clients`, the fallbacks in `_extract_player_responses` | `video/plan.rs` |
| `_video.py` `_extract_player_response`, `_generate_player_context` | `video/player.rs` |
| `_video.py` `_is_agegated`, the "no formats" reasons | `video/playability.rs` |
| `_video.py` `_extract_formats_and_subtitles`, `_get_available_at_timestamp` | `video/formats.rs` |
| `_video.py` player URL, variants, `_extract_signature_timestamp` | `video/player_js.rs` |
| `_video.py` `_download_initial_webpage` | `video/webpage.rs` |
| `jsc/provider.py`, `jsc/_director.py` `validate_response` | `jsc/mod.rs` |
| `jsc/_builtin/ejs.py`, `vendor/_info.py` | `jsc/ejs.rs`, `vendor/ejs/` |
| `jsc/_builtin/quickjs.py` | `jsc/quickjs.rs` (in-process, same script) |

Each Rust file starts with the upstream file, the functions and line ranges
it ports, and says where it differs.

## Syncing

```sh
scripts/sync-upstream.sh                 # yt-dlp master, the ejs release it pins
scripts/sync-upstream.sh <commit> [ejs]  # a given commit or ejs version
```

The script needs `git` and `curl`, and runs no Python. SHA3-512 comes from
OpenSSL 3 when it has it; macOS's `/usr/bin/openssl` (LibreSSL) doesn't, and
then the script uses `cargo run -p encore-resolver --example sha3`. It fetches
both the pinned and the target commit of yt-dlp (and of ejs), then prints:

1. the whole diff of each ported file between the two commits (not only the
   old line ranges: moved functions and new helpers show too);
2. `INNERTUBE_CLIENTS` at the target, next to `innertube_clients.json`;
3. the target ejs version and hashes from `vendor/_info.py`. When the
   version changed, or the vendored files don't match those hashes, it
   downloads both bundles from the ejs release, checks their SHA3-512
   against `_info.py` and replaces `vendor/ejs/` (it refuses on a
   mismatch);
4. the player ids in ejs `tests.ts` and yt-dlp `test_ejs_integration.py`.

Then, by hand:

1. Read the diffs. Port what touches Encore's path into the module the table
   above names: client versions and user agents into the JSON (keep `_line`
   right), order and fallbacks into `plan.rs`, new reasons into
   `playability.rs`, URL handling into `formats.rs`. A change Encore doesn't
   follow goes in the list below, with why.
2. Update `upstream.toml`: the commit, the version, the ejs pins, new fixture
   players. If the solver tests changed, update `tests/ejs_players.rs` from
   `tests.ts` and `test_ejs_integration.py`.
3. `cargo test -p encore-resolver` (the corpus is fetched once into
   `$ENCORE_EJS_PLAYERS`; it then runs offline). The bundle hash test fails
   if `vendor/ejs/` and `upstream.toml` disagree.
4. Run the live checks below, then ship it as a normal release (D24): solver
   code only ever reaches users inside a signed Encore.

### Live checks

```sh
C=target/debug/encore-cli
$C resolve 4D7u5KF7SP8 --diagnostics                  # signed out, the default clients
$C resolve 4D7u5KF7SP8 --client web_embedded --diagnostics
$C resolve SZJvDhaSDnc --diagnostics                  # age-restricted: the age message
$C --auth session resolve 4D7u5KF7SP8 --diagnostics   # signed in (reads the stored session)
ENCORE_DATA_DIR=/tmp/e ENCORE_CACHE_DIR=/tmp/e $C --auth anonymous play 52fNscjkST4 --seek 200 --secs 30
```

Leave a few seconds between requests, and stop at the first bot check:
asking again is how an address gets flagged.

## Where Encore differs from yt-dlp, on purpose

- **One client at a time.** yt-dlp asks every default client and merges
  their formats; Encore asks the next only when this one gave no AAC 141/140
  (the best seen wins at the end). Fewer requests per song.
- **No `web`, `web_safari`; no `web_music` signed in.** They are SABR-only
  or need a GVS PO token, and Encore has no PO token source. `web_creator`
  is planned as upstream for age verification and skipped by its PO token
  policy. Premium isn't detected, so the Premium exemptions never apply.
- **`web_embedded` and `tv` close both lists** as later fallbacks. (`tv`
  answers "The page needs to be reloaded" signed out at the pinned commit,
  for yt-dlp too.)
- **The watch page is loaded once per signed-out session** (6 hours, or
  until a bot check), not for every video. Encore never reads the watch
  page's own player response or formats.
- **The player is found through the iframe API**, never the watch page's
  ytcfg, and always the `main` variant (yt-dlp's default).
- **Made for kids** is not looked for in the watch page: any visionOS
  refusal moves on to `web_embedded`.
- **Only https audio formats.** No HLS, DASH manifests, SABR, live or OTF.
- **The solver runs in-process** on QuickJS-ng with memory, stack and time
  limits, a fresh runtime per solve, and the preprocessed player kept in
  memory (yt-dlp keeps none).
- **Failures are kinds** (D4), not yt-dlp's error text.
- **A preroll wait over 15 s** makes Encore try the next client first.
