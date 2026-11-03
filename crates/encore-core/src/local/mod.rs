//! Local data of the feature wave after 0.4.0 (schema 3, D11), in the same
//! SQLite file as the rest.
//!
//! Two kinds of table, kept apart on purpose:
//!
//! * **Durable**: things the user made or did on this computer. Bookmarks,
//!   saved queues, folders, the block list, shuffle cycles, the play log and
//!   the Last.fm scrobble queue. Every row has an `owner` (see [`Owner`]),
//!   captured when the operation started. Sign-out and channel switches
//!   never touch them (`Store::purge_scope` only clears the `kv` cache);
//!   another owner's rows are simply not shown. They are what the rotating
//!   backups (`Store::backup`) exist for.
//! * **Caches**: answers from elsewhere that can be fetched again: Last.fm
//!   play counts, Wikipedia extracts, release years, audio analysis. They
//!   are per device, never per account, and may be cleared at any time.
//!
//! Nothing secret goes in any of them: no cookies, tokens, feedback tokens,
//! signed URLs or Last.fm keys (D9).
//!
//! Each submodule holds the types for one lane's tables; the schema itself
//! is here so that only one migration ever creates them.

pub mod integrations;
pub mod library;
pub mod playback;
pub mod plays;
pub mod saved;
pub mod taste;

use crate::service::RequestContext;

/// Who a durable row belongs to: the account key (D2) of the request
/// context an operation started under, `anonymous` when signed out. Take it
/// once, at the start (`Owner::of(&svc.context())`), and pass it along:
/// never read the current account again inside a callback or after an
/// `await`, or a switch halfway through files the row under the wrong owner.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Owner(String);

impl Owner {
    pub const ANONYMOUS: &'static str = "anonymous";

    pub fn of(ctx: &RequestContext) -> Self {
        Owner(ctx.account.clone())
    }

    /// An owner key read back from a row, or from `RequestContext::account`.
    pub fn new(key: impl Into<String>) -> Self {
        Owner(key.into())
    }

    pub fn anonymous() -> Self {
        Owner(Self::ANONYMOUS.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_anonymous(&self) -> bool {
        self.0 == Self::ANONYMOUS
    }
}

impl std::fmt::Display for Owner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Schema 3. Times are unix seconds; JSON columns hold the serialised
/// `encore_core::model` types named beside them.
pub(crate) const SCHEMA: &str = r#"
-- Durable --------------------------------------------------------------

-- The play log (D30, lane D; written by encore-player's observer).
CREATE TABLE IF NOT EXISTS plays (
    id         TEXT PRIMARY KEY,           -- one play, a v4 UUID
    owner      TEXT NOT NULL,
    video_id   TEXT NOT NULL,              -- the version that played
    started    INTEGER NOT NULL,
    utc_offset INTEGER NOT NULL DEFAULT 0, -- seconds east of UTC when it started
    duration   REAL,
    heard      REAL NOT NULL DEFAULT 0,    -- audible seconds, seeks and pauses excluded
    counted    INTEGER NOT NULL DEFAULT 0, -- heard >= min(duration / 2, 4 min)
    ended      INTEGER,                    -- set once the play is over
    offline    INTEGER NOT NULL DEFAULT 0,
    private    INTEGER NOT NULL DEFAULT 0, -- played in a private session (D36)
    source     TEXT                        -- QueueSource.collection_id
);
CREATE INDEX IF NOT EXISTS plays_owner_started ON plays(owner, started);
CREATE INDEX IF NOT EXISTS plays_owner_video ON plays(owner, video_id);
-- The last-seen metadata of every played song, so stats never need the
-- network. Per device: `liked` is always cleared (it depends on the account).
CREATE TABLE IF NOT EXISTS track_meta (
    video_id TEXT PRIMARY KEY,
    track    TEXT NOT NULL,                -- Track
    updated  INTEGER NOT NULL
);

-- Bookmarks (D25, lane C).
CREATE TABLE IF NOT EXISTS bookmarks (
    id       TEXT PRIMARY KEY,
    owner    TEXT NOT NULL,
    kind     TEXT NOT NULL,                -- page | track | moment
    target   TEXT NOT NULL,                -- PageRef (page) or Track (track, moment)
    position REAL,                         -- moment: seconds into the song
    title    TEXT NOT NULL,
    subtitle TEXT NOT NULL DEFAULT '',
    art      TEXT,                         -- ArtRef
    created  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS bookmarks_owner ON bookmarks(owner, created);

-- Saved queues (D33, lane C; restored by the player, lane A).
CREATE TABLE IF NOT EXISTS saved_queues (
    id       TEXT PRIMARY KEY,
    owner    TEXT NOT NULL,
    name     TEXT NOT NULL,
    queue    TEXT NOT NULL,                -- encore_core::queue::Queue: order, uids, index
    position REAL NOT NULL DEFAULT 0,      -- in the current song
    songs    INTEGER NOT NULL DEFAULT 0,
    duration REAL,                         -- known total, for the list
    created  INTEGER NOT NULL,
    updated  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS saved_queues_owner ON saved_queues(owner, updated);

-- Shuffle cycles per collection (D33, lane A).
CREATE TABLE IF NOT EXISTS shuffle_cycles (
    owner         TEXT NOT NULL,
    collection_id TEXT NOT NULL,
    cycle         INTEGER NOT NULL DEFAULT 1,
    played        TEXT NOT NULL DEFAULT '[]', -- video ids already played this cycle
    updated       INTEGER NOT NULL,
    PRIMARY KEY (owner, collection_id)
);

-- Folders of playlists and albums (D34, lane C).
CREATE TABLE IF NOT EXISTS folders (
    id        TEXT PRIMARY KEY,
    owner     TEXT NOT NULL,
    name      TEXT NOT NULL,
    cover     TEXT,                        -- opaque local art key, never a path
    position  INTEGER NOT NULL,
    collapsed INTEGER NOT NULL DEFAULT 0,
    created   INTEGER NOT NULL,
    updated   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS folders_owner ON folders(owner, position);
CREATE TABLE IF NOT EXISTS folder_items (
    folder_id TEXT NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
    item_id   TEXT NOT NULL,               -- the playlist or album (ItemId)
    card      TEXT NOT NULL,               -- CollectionCard, shown offline
    position  INTEGER NOT NULL,
    added     INTEGER NOT NULL,
    PRIMARY KEY (folder_id, item_id)
);

-- Songs and artists that don't play (D36, lane D).
CREATE TABLE IF NOT EXISTS blocked (
    owner   TEXT NOT NULL,
    kind    TEXT NOT NULL,                 -- track | artist
    target  TEXT NOT NULL,                 -- ItemId
    label   TEXT NOT NULL,                 -- title or name, for the list in Settings
    detail  TEXT,                          -- the artist line of a song
    created INTEGER NOT NULL,
    PRIMARY KEY (owner, kind, target)
);

-- Plays waiting to be scrobbled (D31, lane E). One per play.
CREATE TABLE IF NOT EXISTS scrobble_queue (
    play_id      TEXT PRIMARY KEY,         -- plays.id
    owner        TEXT NOT NULL,            -- the Encore owner it was queued under
    username     TEXT NOT NULL,            -- the Last.fm account it is for
    artist       TEXT NOT NULL,
    track        TEXT NOT NULL,
    album        TEXT,
    album_artist TEXT,
    duration     REAL,
    timestamp    INTEGER NOT NULL,         -- when the play started
    state        TEXT NOT NULL DEFAULT 'queued', -- queued | sent | ignored | failed
    attempts     INTEGER NOT NULL DEFAULT 0,
    error        TEXT,
    updated      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS scrobble_queue_state ON scrobble_queue(state, timestamp);

-- Caches -----------------------------------------------------------------

-- Last.fm play counts from user.getTopTracks (D31, lane E).
CREATE TABLE IF NOT EXISTS lastfm_plays (
    username   TEXT NOT NULL,
    artist_key TEXT NOT NULL,              -- normalised artist
    title_key  TEXT NOT NULL,              -- normalised title
    plays      INTEGER NOT NULL,
    updated    INTEGER NOT NULL,
    PRIMARY KEY (username, artist_key, title_key)
);

-- Wikipedia extracts for artist pages (D29, lane E). `title` NULL: no
-- article matched.
CREATE TABLE IF NOT EXISTS wiki_cache (
    artist_id   TEXT PRIMARY KEY,
    lang        TEXT NOT NULL,
    title       TEXT,
    description TEXT,
    extract     TEXT,
    url         TEXT,
    fetched     INTEGER NOT NULL
);

-- Estimated tempo and key of a downloaded or cached file (D28, lane E).
CREATE TABLE IF NOT EXISTS audio_analysis (
    video_id   TEXT PRIMARY KEY,
    source     TEXT NOT NULL,              -- which file: "<itag>:<bytes>"
    version    INTEGER NOT NULL,           -- of the estimator
    bpm        REAL,
    music_key  TEXT,                       -- "F# minor"
    confidence REAL,
    analysed   INTEGER NOT NULL
);

-- Release years of albums, for sorting a playlist by year (D34, lane B).
-- `year` NULL: the album gives none.
CREATE TABLE IF NOT EXISTS album_years (
    album_id TEXT PRIMARY KEY,
    year     INTEGER,
    fetched  INTEGER NOT NULL
);
"#;

/// Tables that hold the user's own data (see the module docs).
pub const DURABLE: &[&str] = &["plays", "track_meta", "bookmarks", "saved_queues", "shuffle_cycles", "folders", "folder_items", "blocked", "scrobble_queue"];

/// Tables that only cache what can be fetched or computed again.
pub const CACHES: &[&str] = &["lastfm_plays", "wiki_cache", "audio_analysis", "album_years"];
