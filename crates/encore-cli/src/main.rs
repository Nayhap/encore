//! Headless Encore: exercises the backend crates without Tauri.
//!
//! Output never contains cookie values, SAPISID hashes, tokens or signed
//! stream URLs (everything printed goes through the redactor).

mod show;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use clap::{Parser, Subcommand, ValueEnum};
use encore_audio::{Engine, EngineEvent, SourceSpec};
use encore_core::model::*;
use encore_core::service::MusicService;
use encore_ytm::{InnerTube, YtMusic};

#[derive(Parser)]
#[command(name = "encore-cli", about = "Headless Encore backend tools")]
struct Cli {
    /// Print JSON instead of a summary.
    #[arg(long, global = true)]
    json: bool,
    /// Max rows to print per list.
    #[arg(long, global = true, default_value_t = 8)]
    max: usize,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Clone, Copy, ValueEnum)]
enum Filter {
    Songs,
    Videos,
    Albums,
    Artists,
    Playlists,
    Featured,
}

#[derive(Subcommand)]
enum Cmd {
    /// Decode and play a local audio file through the default output device.
    PlayFile {
        path: PathBuf,
        #[arg(long, default_value_t = 0.0)]
        seek: f64,
        #[arg(long)]
        max_secs: Option<f64>,
    },
    /// Home feed.
    Home {
        /// Also load the next page of shelves.
        #[arg(long)]
        more: bool,
    },
    /// Explore page.
    Explore,
    /// Any browse page (moods, charts, new releases).
    Browse { browse_id: String, #[arg(long)] params: Option<String> },
    Search {
        query: String,
        #[arg(long, value_enum)]
        filter: Option<Filter>,
    },
    Suggest { query: String },
    Album { id: String },
    /// Show a playlist, or change one of yours (signed in).
    #[command(args_conflicts_with_subcommands = true)]
    Playlist {
        #[command(subcommand)]
        edit: Option<PlaylistCmd>,
        id: Option<String>,
        /// Follow continuations until the whole playlist is loaded.
        #[arg(long)]
        all: bool,
    },
    /// Your listening history (signed in), or remove a play from it.
    History {
        #[command(subcommand)]
        cmd: Option<HistoryCmd>,
    },
    /// The other version of a track (song ↔ music video).
    Counterpart { video_id: String },
    Artist { id: String },
    /// Up next / radio for a track (or a playlist with --playlist).
    Radio {
        id: String,
        #[arg(long)]
        playlist: bool,
        #[arg(long)]
        more: bool,
    },
    /// Resolve a stream URL for a video (prints format details, never the URL query).
    /// Installs the player components first if they are missing.
    Resolve { video_id: String },
    /// The player components (Python, Deno, yt-dlp): `status`, `install`, `remove`.
    Runtimes { action: Option<String> },
    /// Stream a video through the engine (resolver + range cache + preload).
    Play {
        video_id: String,
        /// Seek here after playback has started.
        #[arg(long)]
        seek: Option<f64>,
        /// Queue this video next (exercises preload and the gapless hand-over).
        #[arg(long)]
        next: Option<String>,
        /// Stop after this many seconds of wall time.
        #[arg(long, default_value_t = 20.0)]
        secs: f64,
        /// Report the play to YouTube Music history (signed in) and check
        /// that it shows up. Prints ping kinds and statuses, never URLs.
        #[arg(long)]
        report: bool,
        /// Play downloaded songs only, as Encore does offline: everything
        /// comes from the downloads index, with no player components and no
        /// request to YouTube.
        #[arg(long)]
        offline: bool,
    },
    /// Downloads kept in the app's folder (D23).
    Downloads {
        #[command(subcommand)]
        cmd: DownloadsCmd,
    },
    /// Sign-in and session commands.
    Auth {
        #[command(subcommand)]
        cmd: AuthCmd,
    },
    /// Show the signed-in account and channels (names only).
    Whoami,
    /// Library sections (signed in).
    Library {
        #[arg(value_enum, default_value_t = Lib::Playlists)]
        section: Lib,
        /// Follow continuations to load everything.
        #[arg(long)]
        all: bool,
    },
    /// Liked music (signed in), all pages.
    Liked,
    /// Your playlists (signed in).
    Playlists,
    /// Debug: POST an InnerTube endpoint (with the session if signed in) and
    /// print the renderer key tree (no values).
    Raw {
        endpoint: String,
        body: String,
        #[arg(long)]
        signed: bool,
        #[arg(long, default_value_t = 9)]
        depth: usize,
    },
    /// Lyrics for a video via the provider chain (and the SQLite cache).
    Lyrics {
        video_id: String,
        /// Skip the cache.
        #[arg(long)]
        fresh: bool,
    },
    /// Record scrubbed fixtures into crates/encore-ytm/fixtures.
    Record { dir: Option<PathBuf> },
}

#[derive(Subcommand)]
enum AuthCmd {
    /// List browser profiles that can be imported (no cookie store is opened).
    Profiles,
    /// Import the YouTube session from Microsoft Edge (macOS asks for Keychain access).
    ImportEdge {
        #[arg(long, default_value = "Default")]
        profile: String,
        /// Seconds to wait for a Keychain prompt (macOS).
        #[arg(long, default_value_t = 18)]
        timeout: u64,
    },
    /// Import from any browser profile id ("chrome:Default", "firefox:xyz", "safari:default").
    Import {
        profile_id: String,
        #[arg(long, default_value_t = 18)]
        timeout: u64,
    },
    /// Choose the channel (key "authUser:pageId" or list index).
    Select { channel: String },
    /// Run one keep-alive round.
    Keepalive,
    /// Delete the stored session.
    SignOut,
}

#[derive(Clone, Copy, ValueEnum)]
enum Vis {
    Private,
    Unlisted,
    Public,
}

impl From<Vis> for Privacy {
    fn from(v: Vis) -> Self {
        match v {
            Vis::Private => Privacy::Private,
            Vis::Unlisted => Privacy::Unlisted,
            Vis::Public => Privacy::Public,
        }
    }
}

#[derive(Subcommand)]
enum PlaylistCmd {
    /// Create a playlist (private unless --privacy says otherwise).
    Create {
        title: String,
        #[arg(long, default_value = "")]
        description: String,
        #[arg(long, value_enum, default_value_t = Vis::Private)]
        privacy: Vis,
        /// Songs to start it with.
        #[arg(long = "add")]
        add: Vec<String>,
    },
    Rename { id: String, title: String },
    Describe { id: String, description: String },
    Privacy {
        id: String,
        #[arg(value_enum)]
        privacy: Vis,
    },
    /// Add songs; refused as a duplicate unless --anyway.
    Add {
        id: String,
        videos: Vec<String>,
        #[arg(long)]
        anyway: bool,
    },
    /// Move the row at index FROM before the row at --before (or to the end).
    Move {
        id: String,
        from: usize,
        #[arg(long)]
        before: Option<usize>,
    },
    Delete { id: String },
}

#[derive(Subcommand)]
enum DownloadsCmd {
    /// Downloaded songs, kept collections and what is waiting.
    List,
    /// Download songs and wait until they are done (installs the player
    /// components first if missing).
    Add { videos: Vec<String> },
    /// Keep a playlist, album or liked songs (LM) downloaded, and wait.
    Keep { id: String },
    /// Stop keeping a collection downloaded.
    Unkeep { id: String },
    /// Remove downloaded songs (kept collections keep theirs).
    Remove { videos: Vec<String> },
    /// Check kept collections for changes, and download what's new.
    Sync { id: Option<String> },
    Pause,
    /// Resume, and try failed songs again.
    Resume,
}

#[derive(Subcommand)]
enum HistoryCmd {
    /// Remove the most recent play of a video from the history.
    Remove { video_id: String },
    /// Is this video in the newest day group?
    Has { video_id: String },
}

#[derive(Clone, Copy, ValueEnum)]
enum Lib {
    Playlists,
    Albums,
    Artists,
    Songs,
    Subscriptions,
    History,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Cmd::PlayFile { path, seek, max_secs } = &cli.cmd {
        return play_file(path.clone(), *seek, *max_secs);
    }
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    rt.block_on(run(cli))
}

fn out<T: serde::Serialize>(json: bool, v: &T, f: impl FnOnce(&T)) {
    if json {
        let s = serde_json::to_string_pretty(v).unwrap_or_default();
        println!("{}", encore_core::redact::redact(&s));
    } else {
        f(v)
    }
}

fn ytm() -> Arc<YtMusic> {
    Arc::new(YtMusic::new(InnerTube::new("en", "US")))
}

async fn run(cli: Cli) -> Result<()> {
    let svc = ytm();
    let ctx = svc.context();
    let m = cli.max;
    let j = cli.json;
    let e = |e: encore_core::AppError| anyhow!("{} ({:?})", e.message, e.code);
    match cli.cmd {
        Cmd::PlayFile { .. } => unreachable!(),
        Cmd::Home { more } => {
            let p = svc.home(&ctx).await.map_err(e)?;
            out(j, &p, |p| show::page(p, m));
            if more {
                if let Some(c) = &p.cursor {
                    let p2 = svc.more_shelves(&ctx, c).await.map_err(e)?;
                    println!("\n--- next page ---");
                    out(j, &p2, |p| show::page(p, m));
                }
            }
        }
        Cmd::Explore => {
            let p = svc.explore(&ctx).await.map_err(e)?;
            out(j, &p, |p| show::page(p, m));
        }
        Cmd::Browse { browse_id, params } => {
            let p = svc.browse_page(&ctx, &browse_id, params.as_deref()).await.map_err(e)?;
            out(j, &p, |p| show::page(p, m));
        }
        Cmd::Search { query, filter } => {
            let f = filter.map(|f| match f {
                Filter::Songs => SearchFilter::Songs,
                Filter::Videos => SearchFilter::Videos,
                Filter::Albums => SearchFilter::Albums,
                Filter::Artists => SearchFilter::Artists,
                Filter::Playlists => SearchFilter::CommunityPlaylists,
                Filter::Featured => SearchFilter::FeaturedPlaylists,
            });
            let r = svc.search(&ctx, &query, f).await.map_err(e)?;
            out(j, &r, |r| show::search(r, m));
        }
        Cmd::Suggest { query } => {
            let r = svc.suggestions(&ctx, &query).await.map_err(e)?;
            out(j, &r, |r| r.iter().for_each(|s| println!("{s}")));
        }
        Cmd::Playlist { edit: Some(cmd), .. } => {
            accounts(&svc).restore().map_err(e)?;
            playlist_edit(&svc, cmd).await?;
        }
        Cmd::Playlist { id: None, .. } => bail!("give a playlist id, or a change such as `playlist create <title>`"),
        Cmd::History { cmd } => {
            accounts(&svc).restore().map_err(e)?;
            history(&svc, cmd, m, j).await?;
        }
        Cmd::Counterpart { video_id } => {
            let _ = accounts(&svc).restore();
            let ctx = svc.context();
            let t = svc.track(&ctx, &ItemId::parse(&video_id)).await.map_err(e)?;
            println!("track:       {}", show::track(&t));
            match svc.counterpart(&ctx, &t).await.map_err(e)? {
                None => println!("counterpart: none"),
                Some(c) => {
                    println!("counterpart: {}", show::track(&c.track));
                    for s in &c.segments {
                        println!("  shared: {:.3}s here = {:.3}s there, for {:.3}s", s.from_start, s.to_start, s.duration);
                    }
                    if c.segments.is_empty() {
                        println!("  no segment map: a switch restarts at 0");
                    }
                }
            }
        }
        Cmd::Album { id } | Cmd::Playlist { id: Some(id), all: false, .. } => {
            let c = svc.collection(&ctx, &ItemId::parse(&id)).await.map_err(e)?;
            out(j, &c, |c| show::collection(c, m));
            println!("entries: {}  more pages: {}  editable: {}  privacy: {:?}", c.entries.len(), c.cursor.is_some(), c.editable, c.privacy);
        }
        Cmd::Playlist { id: Some(id), all: true, .. } => {
            let mut c = svc.collection(&ctx, &ItemId::parse(&id)).await.map_err(e)?;
            let mut pages = 1;
            let mut cur = c.cursor.clone();
            while let Some(cid) = cur {
                let (more, next) = svc.collection_more(&ctx, &cid).await.map_err(e)?;
                pages += 1;
                c.entries.extend(more);
                cur = next;
                if pages > 200 {
                    break;
                }
            }
            c.cursor = None;
            out(j, &c, |c| show::collection(c, m));
            let mut uniq = std::collections::HashSet::new();
            let distinct = c.entries.iter().filter(|e| uniq.insert(e.entry_id.clone())).count();
            println!("pages: {pages}  entries: {}  distinct entry ids: {distinct}", c.entries.len());
        }
        Cmd::Artist { id } => {
            let a = svc.artist(&ctx, &ItemId::parse(&id)).await.map_err(e)?;
            out(j, &a, |a| show::artist(a, m));
        }
        Cmd::Radio { id, playlist, more } => {
            let seed = if playlist { RadioSeed::Playlist { playlist_id: id } } else { RadioSeed::Track { id: ItemId::parse(&id) } };
            let r = svc.up_next(&ctx, &seed, None).await.map_err(e)?;
            out(j, &r, |r| {
                println!("playlist: {}", r.playlist_id.as_deref().unwrap_or("-"));
                for t in r.tracks.iter().take(m) {
                    println!("  {}", show::track(t));
                }
                println!("tracks: {}  more: {}", r.tracks.len(), r.cursor.is_some());
            });
            if more {
                if let Some(c) = &r.cursor {
                    let r2 = svc.up_next(&ctx, &seed, Some(c)).await.map_err(e)?;
                    println!("next page: {} tracks, more: {}", r2.tracks.len(), r2.cursor.is_some());
                }
            }
        }
        Cmd::Record { dir } => record(&svc, dir).await?,
        Cmd::Play { video_id, seek, next, secs, report, offline } => {
            let _ = accounts(&svc).restore();
            play(svc.clone(), &video_id, seek, next.as_deref(), secs, report, offline).await?
        }
        Cmd::Downloads { cmd } => {
            let _ = accounts(&svc).restore();
            downloads(svc.clone(), cmd, j).await?
        }
        Cmd::Auth { cmd } => auth(&svc, cmd).await?,
        Cmd::Lyrics { video_id, fresh } => {
            let id = ItemId::parse(&video_id);
            let track = svc.track(&ctx, &id).await.map_err(e)?;
            let db = dirs::data_local_dir().unwrap_or_else(std::env::temp_dir).join("app.encore.desktop/encore.sqlite");
            let store = Arc::new(encore_core::store::Store::open(&db).map_err(e)?);
            let ls = encore_lyrics::LyricsService::new(svc.clone(), Some(store.clone()));
            if fresh {
                ls.forget(&id);
            }
            let t0 = Instant::now();
            let l = ls.get(&track).await;
            println!("track: {}", show::track(&track));
            match l {
                None => println!("no lyrics found ({:.2}s)", t0.elapsed().as_secs_f64()),
                Some(l) => {
                    out(j, &l, |l| {
                        println!("source: {}  sync: {:?}  lines: {}  ({:.2}s){}", l.source, l.sync, l.lines.len(), t0.elapsed().as_secs_f64(), l.attribution.as_ref().map(|a| format!("  [{a}]")).unwrap_or_default());
                        for x in l.lines.iter().take(m) {
                            println!("  [{}] {}", show::dur(Some(x.t)), x.text);
                        }
                        if let Some(p) = &l.plain {
                            for x in p.lines().take(m) {
                                println!("  {x}");
                            }
                        }
                    });
                }
            }
        }
        Cmd::Raw { endpoint, body, signed, depth } => {
            if signed {
                accounts(&svc).restore().map_err(e)?;
            }
            let b: serde_json::Value = serde_json::from_str(&body)?;
            let v = svc.raw(&endpoint, b).await.map_err(e)?;
            fn walk(v: &serde_json::Value, d: usize, max: usize) {
                if d > max {
                    return;
                }
                match v {
                    serde_json::Value::Object(m) => {
                        for (k, x) in m {
                            if std::env::var("ENCORE_RAW_ALL").is_ok() || k.ends_with("Renderer") || k.ends_with("Model") || ["contents", "items", "tabs", "header", "continuations", "continuationContents", "onResponseReceivedActions", "content"].contains(&k.as_str()) {
                                let n = x.as_array().map(|a| format!("[{}]", a.len())).unwrap_or_default();
                                println!("{}{k}{n}", "  ".repeat(d));
                                walk(x, d + 1, max);
                            }
                        }
                    }
                    serde_json::Value::Array(a) => {
                        let mut seen = std::collections::HashSet::new();
                        for x in a {
                            let sig = x.as_object().map(|m| m.keys().cloned().collect::<Vec<_>>().join(",")).unwrap_or_default();
                            if seen.insert(sig) {
                                walk(x, d, max);
                            }
                        }
                    }
                    _ => {}
                }
            }
            walk(&v, 0, depth);
        }
        Cmd::Whoami => {
            let acc = accounts(&svc);
            let st = acc.restore().map_err(e)?;
            if matches!(st, AuthState::Anonymous) {
                println!("not signed in (anonymous). Run: encore-cli auth import-edge");
                return Ok(());
            }
            let s = svc.session().unwrap();
            println!("session source: {}  cookies: {} (values hidden)  created: {}", s.source, s.jar.len(), s.created_at);
            match acc.keepalive().await {
                Ok(changed) => println!("verify_session: ok (rotated: {changed})"),
                Err(err) => println!("verify_session: {}", err.message),
            }
            let chans = acc.load_channels().await.map_err(e)?;
            let state = acc.state();
            let sel = match &state {
                AuthState::SignedIn { channel, .. } => Some(channel.key.clone()),
                _ => None,
            };
            println!("channels:");
            for (i, c) in chans.iter().enumerate() {
                println!("  [{i}] {}{}{}  key={}{}", c.name, c.handle.as_ref().map(|h| format!(" ({h})")).unwrap_or_default(), if c.is_brand { " [brand]" } else { "" }, c.key, if sel.as_deref() == Some(c.key.as_str()) { "  <- selected" } else { "" });
            }
            match state {
                AuthState::SignedIn { channel, .. } => println!("signed in as: {}", channel.name),
                AuthState::ChoosingChannel { .. } => println!("no channel chosen yet. Run: encore-cli auth select <key or index>"),
                AuthState::Expired { reason, .. } => println!("session expired: {reason}"),
                AuthState::Anonymous => {}
            }
        }
        Cmd::Library { section, all } => {
            accounts(&svc).restore().map_err(e)?;
            let ctx = svc.context();
            let sec = match section {
                Lib::Playlists => LibrarySection::Playlists,
                Lib::Albums => LibrarySection::Albums,
                Lib::Artists => LibrarySection::Artists,
                Lib::Songs => LibrarySection::Songs,
                Lib::Subscriptions => LibrarySection::Subscriptions,
                Lib::History => LibrarySection::History,
            };
            library(&svc, &ctx, sec, all, m, j).await?;
        }
        Cmd::Liked => {
            accounts(&svc).restore().map_err(e)?;
            let ctx = svc.context();
            library(&svc, &ctx, LibrarySection::Liked, true, m, j).await?;
        }
        Cmd::Playlists => {
            accounts(&svc).restore().map_err(e)?;
            let ctx = svc.context();
            library(&svc, &ctx, LibrarySection::Playlists, true, m, j).await?;
        }
        Cmd::Runtimes { action } => {
            let r = cli_resolver(false).await?;
            match action.as_deref().unwrap_or("status") {
                "status" => {}
                "install" => install_runtimes(r.runtimes()).await?,
                "remove" => r.runtimes().remove(false).await.map_err(|e| anyhow!("{}", e.message))?,
                other => bail!("unknown action {other}; use status, install or remove"),
            }
            let st = r.runtimes().status_measured();
            println!("location: {}", st.location);
            println!("ready={} usable={} approved={} download={:.1} MB disk≈{:.0} MB used={:.1} MB", st.ready, st.usable, st.approved, st.download_bytes / 1e6, st.disk_bytes / 1e6, st.used_bytes / 1e6);
            for c in &st.components {
                println!("  {} {} (installed: {}){}", c.name, c.version, c.installed.as_deref().unwrap_or("none"), if c.ready { "" } else { " needs install" });
            }
            if let Some(m) = st.message {
                println!("{m}");
            }
        }
        Cmd::Resolve { video_id } => {
            use encore_core::service::StreamResolver;
            let t0 = Instant::now();
            let r = cli_resolver(true).await?;
            let st = r.runtimes().status();
            println!("components: {} (setup {:.1}s)", st.components.iter().map(|c| format!("{} {}", c.name, c.installed.as_deref().unwrap_or("?"))).collect::<Vec<_>>().join(", "), t0.elapsed().as_secs_f64());
            let t1 = Instant::now();
            let s = r.resolve(&ItemId::parse(&video_id).remote().to_string(), None).await.map_err(|e| anyhow!("{} ({:?})", e.message, e.kind))?;
            let host = url::Url::parse(s.url.expose()).ok().and_then(|u| u.host_str().map(|h| h.to_string())).unwrap_or_default();
            println!(
                "resolved {} in {:.2}s: itag={} codec={} mime={} kbps={} bytes={} duration={} range_safe={} authenticated={} expires_in={}s host={} url=<redacted>",
                s.video_id,
                t1.elapsed().as_secs_f64(),
                s.itag,
                s.codec,
                s.mime,
                s.bitrate_kbps.map(|b| b.to_string()).unwrap_or("?".into()),
                s.content_length.map(|b| b.to_string()).unwrap_or("?".into()),
                show::dur(s.duration),
                s.range_safe,
                s.authenticated,
                s.expires_at.map(|e| e - encore_core::store::now()).unwrap_or(0),
                host
            );
            println!("probe: head range ok, tail range {}", if s.range_safe { "ok" } else { "failed (sequential mode)" });
        }
    }
    Ok(())
}

fn accounts(svc: &Arc<YtMusic>) -> Arc<encore_ytm::account::Accounts> {
    encore_ytm::account::Accounts::new(svc.clone(), encore_auth::store::session_store())
}

async fn library(svc: &Arc<YtMusic>, ctx: &encore_core::service::RequestContext, sec: LibrarySection, all: bool, m: usize, j: bool) -> Result<()> {
    let e = |e: encore_core::AppError| anyhow!("{} ({:?})", e.message, e.code);
    let mut page = svc.library(ctx, sec).await.map_err(e)?;
    let mut pages = 1;
    while all && page.cursor.is_some() && pages < 200 {
        let c = page.cursor.take().unwrap();
        let more = svc.library_more(ctx, sec, &c).await.map_err(e)?;
        pages += 1;
        page.items.extend(more.items);
        page.cursor = more.cursor;
    }
    out(j, &page, |p| {
        for i in p.items.iter().take(m) {
            println!("  {}", show::item(i));
        }
        println!("{:?}: {} items over {pages} page(s); more: {}", p.section, p.items.len(), p.cursor.is_some());
    });
    Ok(())
}

async fn auth(svc: &Arc<YtMusic>, cmd: AuthCmd) -> Result<()> {
    let e = |e: encore_core::AppError| anyhow!("{} ({:?})", e.message, e.code);
    let acc = accounts(svc);
    let print_state = |st: &AuthState| match st {
        AuthState::ChoosingChannel { channels, source } => {
            println!("imported from {source}; choose a channel:");
            for (i, c) in channels.iter().enumerate() {
                println!("  [{i}] {}{}  key={}", c.name, if c.is_brand { " [brand]" } else { "" }, c.key);
            }
            println!("then run: encore-cli auth select <index or key>");
        }
        AuthState::SignedIn { channel, .. } => println!("signed in as {}", channel.name),
        AuthState::Expired { reason, .. } => println!("expired: {reason}"),
        AuthState::Anonymous => println!("anonymous"),
    };
    match cmd {
        AuthCmd::Profiles => {
            for p in encore_auth::import::list_profiles() {
                println!("{}  {} — {}", p.id, p.browser, p.name);
            }
        }
        AuthCmd::ImportEdge { profile, timeout } => {
            if cfg!(target_os = "macos") {
                println!("Reading the \"Microsoft Edge Safe Storage\" key from the Keychain (macOS may ask you to allow this)…");
            }
            let st = acc.import_browser(&format!("edge:{profile}"), Duration::from_secs(timeout)).await.map_err(e)?;
            let n = svc.session().map(|s| s.jar.len()).unwrap_or(0);
            println!("imported {n} cookies (values hidden), saved to {} under \"app.encore.desktop\"", encore_auth::store::STORE_NAME);
            print_state(&st);
        }
        AuthCmd::Import { profile_id, timeout } => {
            let st = acc.import_browser(&profile_id, Duration::from_secs(timeout)).await.map_err(e)?;
            print_state(&st);
        }
        AuthCmd::Select { channel } => {
            acc.restore().map_err(e)?;
            let st = acc.select_channel(&channel).await.map_err(e)?;
            print_state(&st);
        }
        AuthCmd::Keepalive => {
            acc.restore().map_err(e)?;
            let changed = acc.keepalive().await.map_err(e)?;
            println!("verify_session ok; cookies rotated: {changed}");
        }
        AuthCmd::SignOut => {
            acc.sign_out().map_err(e)?;
            println!("signed out; removed from {}", encore_auth::store::STORE_NAME);
        }
    }
    Ok(())
}

async fn record(svc: &YtMusic, dir: Option<PathBuf>) -> Result<()> {
    use serde_json::json;
    let dir = dir.unwrap_or_else(|| PathBuf::from("crates/encore-ytm/fixtures"));
    std::fs::create_dir_all(&dir)?;
    let save = |name: &str, mut v: serde_json::Value| -> Result<()> {
        encore_ytm::scrub::scrub(&mut v);
        let s = serde_json::to_string(&v)?;
        // Belt and braces: the redactor must not find anything to change.
        let red = encore_core::redact::redact(&s);
        if red != s {
            let i = s.bytes().zip(red.bytes()).position(|(a, b)| a != b).unwrap_or(0);
            let ctx: String = red[i.saturating_sub(60)..(i + 20).min(red.len())].to_string();
            bail!("fixture {name} still contains secret-looking data after scrubbing near: {ctx}");
        }
        std::fs::write(dir.join(format!("{name}.json")), s)?;
        println!("recorded {name}");
        Ok(())
    };
    let e = |e: encore_core::AppError| anyhow!("{}", e.message);
    save("home", svc.raw("browse", json!({"browseId": "FEmusic_home"})).await.map_err(e)?)?;
    save("explore", svc.raw("browse", json!({"browseId": "FEmusic_explore"})).await.map_err(e)?)?;
    save("moods", svc.raw("browse", json!({"browseId": "FEmusic_moods_and_genres"})).await.map_err(e)?)?;
    save("search_all", svc.raw("search", json!({"query": "daft punk"})).await.map_err(e)?)?;
    save("search_songs", svc.raw("search", json!({"query": "daft punk", "params": "EgWKAQIIAWoMEA4QChADEAQQCRAF"})).await.map_err(e)?)?;
    save("search_videos", svc.raw("search", json!({"query": "daft punk", "params": "EgWKAQIQAWoMEA4QChADEAQQCRAF"})).await.map_err(e)?)?;
    save("suggestions", svc.raw("music/get_search_suggestions", json!({"input": "daft p"})).await.map_err(e)?)?;
    save("album", svc.raw("browse", json!({"browseId": "MPREb_7ltM34kr0mH"})).await.map_err(e)?)?;
    save("artist", svc.raw("browse", json!({"browseId": "UCRr1xG_2WIDs18a6cIiCxeA"})).await.map_err(e)?)?;
    let pl = svc.raw("browse", json!({"browseId": "VLPLgVLQgm4lkANkwLnIYZiwf6o2jKIc5Th3"})).await.map_err(e)?;
    let tok = encore_ytm::pages::collection(&pl, &ItemId::ytm("VLPLgVLQgm4lkANkwLnIYZiwf6o2jKIc5Th3")).ok().and_then(|p| p.cont);
    save("playlist_long", pl)?;
    if let Some(encore_ytm::pages::Cont::Body(t)) = tok {
        save("playlist_continuation", svc.raw("browse", json!({"continuation": t})).await.map_err(e)?)?;
    }
    save(
        "next_radio",
        svc.raw("next", json!({"videoId": "4D7u5KF7SP8", "playlistId": "RDAMVM4D7u5KF7SP8", "isAudioOnly": true, "enablePersistentPlaylistPanel": true, "params": "wAEB"})).await.map_err(e)?,
    )?;
    // The album (Discovery) played as a queue: its items are music videos.
    save(
        "next_album",
        svc.raw("next", json!({"playlistId": "OLAK5uy_mz6eafmqdRHSaR4IwG0ll6J6rgv0_ZpGw", "isAudioOnly": true, "enablePersistentPlaylistPanel": true})).await.map_err(e)?,
    )?;
    save("lyrics_android", svc.raw_android("browse", json!({"browseId": "MPLYt_K8qWMWVqXGi-8"})).await.map_err(e)?)?;
    save("lyrics_web", svc.raw("browse", json!({"browseId": "MPLYt_K8qWMWVqXGi-8"})).await.map_err(e)?)?;
    Ok(())
}

fn outcome(o: MutationOutcome) -> &'static str {
    match o {
        MutationOutcome::Done => "done",
        MutationOutcome::Rejected => "rejected",
        MutationOutcome::OutcomeUnknown => "outcome unknown (refetch before retrying)",
        MutationOutcome::Duplicate => "duplicate (already in the playlist; nothing added)",
    }
}

async fn playlist_edit(svc: &Arc<YtMusic>, cmd: PlaylistCmd) -> Result<()> {
    let e = |e: encore_core::AppError| anyhow!("{} ({:?})", e.message, e.code);
    let ctx = svc.context();
    let pid = |id: &str| id.trim_start_matches("ytm:").to_string();
    match cmd {
        PlaylistCmd::Create { title, description, privacy, add } => {
            let ids: Vec<ItemId> = add.iter().map(|v| ItemId::parse(v)).collect();
            let r = svc.playlist_create(&ctx, &title, &description, privacy.into(), &ids).await.map_err(e)?;
            println!("create: {}", outcome(r.outcome));
            if let Some(id) = r.playlist_id {
                println!("playlist id: {id}");
            }
        }
        PlaylistCmd::Rename { id, title } => {
            let r = svc.playlist_edit(&ctx, &pid(&id), &PlaylistEdit { title: Some(title), ..Default::default() }).await.map_err(e)?;
            println!("rename: {}", outcome(r));
        }
        PlaylistCmd::Describe { id, description } => {
            let r = svc.playlist_edit(&ctx, &pid(&id), &PlaylistEdit { description: Some(description), ..Default::default() }).await.map_err(e)?;
            println!("describe: {}", outcome(r));
        }
        PlaylistCmd::Privacy { id, privacy } => {
            let r = svc.playlist_edit(&ctx, &pid(&id), &PlaylistEdit { privacy: Some(privacy.into()), ..Default::default() }).await.map_err(e)?;
            println!("privacy: {}", outcome(r));
        }
        PlaylistCmd::Add { id, videos, anyway } => {
            let ids: Vec<ItemId> = videos.iter().map(|v| ItemId::parse(v)).collect();
            let r = svc.playlist_add(&ctx, &pid(&id), &ids, anyway).await.map_err(e)?;
            println!("add: {}", outcome(r));
        }
        PlaylistCmd::Move { id, from, before } => {
            let c = svc.collection(&ctx, &ItemId::parse(&format!("VL{}", pid(&id).trim_start_matches("VL")))).await.map_err(e)?;
            let set = |i: usize| c.entries.get(i).and_then(|x| x.set_video_id.clone()).ok_or_else(|| anyhow!("no movable row at index {i}"));
            let s = set(from)?;
            let b = before.map(set).transpose()?;
            let r = svc.playlist_move(&ctx, &pid(&id), &s, b.as_deref()).await.map_err(e)?;
            println!("move: {}", outcome(r));
        }
        PlaylistCmd::Delete { id } => {
            let r = svc.playlist_delete(&ctx, &pid(&id)).await.map_err(e)?;
            println!("delete: {}", outcome(r));
        }
    }
    Ok(())
}

async fn history(svc: &Arc<YtMusic>, cmd: Option<HistoryCmd>, m: usize, j: bool) -> Result<()> {
    let e = |e: encore_core::AppError| anyhow!("{} ({:?})", e.message, e.code);
    let ctx = svc.context();
    let h = svc.history(&ctx).await.map_err(e)?;
    match cmd {
        None => out(j, &h, |h| {
            if let Some(n) = &h.notice {
                println!("notice: {n}{}", if h.paused { " (paused)" } else { "" });
            }
            for g in &h.groups {
                println!("\n== {} ({} plays)", g.title, g.rows.len());
                for r in g.rows.iter().take(m) {
                    println!("  {}{}", show::track(&r.track), if r.removable { "" } else { "  (not removable)" });
                }
            }
            println!("\ngroups: {}  more: {}", h.groups.len(), h.cursor.is_some());
        }),
        Some(HistoryCmd::Has { video_id }) => {
            let id = ItemId::parse(&video_id);
            let first = h.groups.first();
            let yes = first.is_some_and(|g| g.rows.iter().any(|r| r.track.id == id));
            println!("history: {id} in \"{}\": {}", first.map(|g| g.title.as_str()).unwrap_or("-"), if yes { "yes" } else { "no" });
        }
        Some(HistoryCmd::Remove { video_id }) => {
            let id = ItemId::parse(&video_id);
            let Some(row) = h.groups.iter().flat_map(|g| &g.rows).find(|r| r.track.id == id) else { bail!("{id} is not in the history") };
            if !row.removable {
                bail!("YouTube Music offers no removal for that row");
            }
            let r = svc.history_remove(&ctx, std::slice::from_ref(&row.row_id)).await.map_err(e)?;
            println!("remove: {}", outcome(r));
            tokio::time::sleep(Duration::from_secs(3)).await;
            let h = svc.history(&svc.context()).await.map_err(e)?;
            let n = h.groups.iter().flat_map(|g| &g.rows).filter(|r| r.track.id == id).count();
            println!("plays of {id} left in history: {n}");
        }
    }
    Ok(())
}

/// Prints which ping went out and its result; never the URL.
struct ShowReporter(Arc<YtMusic>);
struct ShowReport(Arc<dyn encore_core::service::PlayReport>);

#[async_trait::async_trait]
impl encore_core::service::PlayReporter for ShowReporter {
    async fn open_report(&self, id: &ItemId) -> encore_core::Result<Option<Arc<dyn encore_core::service::PlayReport>>> {
        let r = self.0.open_report(id).await;
        match &r {
            Ok(Some(rep)) => println!("report: /player ok, tracking present (flush at {:?} then every {}s)", rep.schedule().at, rep.schedule().every),
            Ok(None) => println!("report: not reportable (signed out, or no tracking in /player)"),
            Err(err) => println!("report: /player failed: {}", err.message),
        }
        Ok(r?.map(|rep| Arc::new(ShowReport(rep)) as Arc<dyn encore_core::service::PlayReport>))
    }
}

#[async_trait::async_trait]
impl encore_core::service::PlayReport for ShowReport {
    fn schedule(&self) -> encore_core::service::FlushSchedule {
        self.0.schedule()
    }
    async fn send(&self, ping: &encore_core::service::Ping, itag: Option<u32>) -> encore_core::Result<()> {
        let r = self.0.send(ping, itag).await;
        let what = match ping {
            encore_core::service::Ping::Playback { cmt, .. } => format!("playback at {cmt:.1}s"),
            encore_core::service::Ping::Watchtime { st, et, last, .. } => format!("watchtime {}{}", st.iter().zip(et).map(|(a, b)| format!("{a:.1}–{b:.1}")).collect::<Vec<_>>().join(","), if *last { " (final)" } else { "" }),
        };
        println!("ping: {what}: {}", match &r { Ok(()) => "ok".to_string(), Err(e) => e.message.clone() });
        r
    }
}

async fn play(svc: Arc<YtMusic>, video_id: &str, seek: Option<f64>, next: Option<&str>, secs: f64, report: bool, offline: bool) -> Result<()> {
    use encore_player::{Player, PlayerEvent};
    if offline && report {
        bail!("--report needs a connection; leave out --offline");
    }
    // Offline nothing is installed: downloaded songs need no components.
    let resolver = cli_resolver(!offline).await?;
    let cache_dir = dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("app.encore.desktop/audio");
    let cache = encore_audio::AudioCache::new(&cache_dir, encore_audio::cache::DEFAULT_LIMIT)?;
    let it = svc.it.clone();
    let auth: encore_player::AuthProvider = Arc::new(move || encore_cli_auth(&it));
    let player = Player::new(svc.clone(), resolver.clone(), auth, Some(cache.clone())).map_err(|e| anyhow!("{}", e.message))?;
    let mut rx = player.subscribe();
    let ctx = svc.context();
    let target = ItemId::parse(video_id);
    let mut was_there = false;
    if report {
        if !svc.it.signed_in() {
            bail!("--report needs a signed-in session (encore-cli whoami)");
        }
        let h = svc.history(&ctx).await.map_err(|e| anyhow!("{}", e.message))?;
        was_there = h.groups.first().is_some_and(|g| g.rows.iter().any(|r| r.track.id == target));
        println!("history before: {target} in the newest group: {}", if was_there { "yes" } else { "no" });
        player.set_reporting(true);
        player.report_history(Arc::new(ShowReporter(svc.clone())), Arc::new(|id: &ItemId| println!("history: play of {id} registered")));
    }
    let mut tracks = Vec::new();
    if offline {
        // Everything from the downloads index; nothing from YouTube.
        let dl = cli_downloads(svc.clone(), resolver.clone(), Some(cache.clone())).await?;
        for id in std::iter::once(video_id).chain(next) {
            let id = ItemId::parse(id);
            let row = dl.1.download_row(id.remote()).map_err(|e| anyhow!("{}", e.message))?;
            match row {
                Some(r) if r.state == encore_core::downloads::RowState::Ready => tracks.push(r.track),
                _ => println!("not downloaded, left out: {id}"),
            }
        }
        if tracks.is_empty() {
            bail!("none of these songs are downloaded (encore-cli downloads add …)");
        }
        player.set_local_audio(dl.0);
        player.set_online(false);
    } else {
        tracks.push(svc.track(&ctx, &ItemId::parse(video_id)).await.map_err(|e| anyhow!("{}", e.message))?);
        if let Some(n) = next {
            tracks.push(svc.track(&ctx, &ItemId::parse(n)).await.map_err(|e| anyhow!("{}", e.message))?);
        }
    }
    for t in &tracks {
        println!("queue: {}", show::track(t));
    }
    player.set_autoplay(false);
    player.play_tracks(tracks, 0, None, false).map_err(|e| anyhow!("{}", e.message))?;
    let t0 = Instant::now();
    let mut seeked = false;
    let mut last = Instant::now();
    let mut started_at: Option<Instant> = None;
    let mut transitions = 0;
    while t0.elapsed().as_secs_f64() < secs {
        while let Ok(ev) = rx.try_recv() {
            match ev {
                PlayerEvent::TrackChanged(Some(e)) => {
                    transitions += 1;
                    println!("track: {} — {}", e.track.title, e.track.artist_line());
                }
                PlayerEvent::State(s) if s.error.is_some() => println!("error: {:?}", s.error),
                PlayerEvent::Notice(n) => println!("notice: {n}"),
                _ => {}
            }
        }
        let snap = player.snapshot();
        if started_at.is_none() && snap.phase == encore_player::Phase::Playing {
            started_at = Some(Instant::now());
            println!("audible after {:.2}s{} (stream itag={} codec={} cached={} authenticated={} seekable={})", t0.elapsed().as_secs_f64(), if offline { " from downloads" } else { "" }, snap.stream.as_ref().map(|s| s.itag).unwrap_or(0), snap.stream.as_ref().map(|s| s.codec.as_str()).unwrap_or("?"), snap.stream.as_ref().is_some_and(|s| s.from_cache), snap.stream.as_ref().is_some_and(|s| s.authenticated), snap.stream.as_ref().is_some_and(|s| s.seekable));
        }
        if let (Some(target), false, Some(st)) = (seek, seeked, started_at) {
            if st.elapsed() > Duration::from_secs(3) {
                seeked = true;
                println!("seek → {target:.1}s");
                player.seek(target);
            }
        }
        if last.elapsed() > Duration::from_secs(1) {
            last = Instant::now();
            let es = player.engine().status();
            println!(
                "{:?} pos={:.1}/{} frames_out={} callbacks={} underruns={} next_deck={} device=\"{}\"",
                snap.phase,
                snap.position,
                show::dur(snap.duration),
                es.frames_out,
                es.callbacks,
                es.underruns,
                es.next_id.map(|i| i.to_string()).unwrap_or("-".into()),
                es.device
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    println!("track changes: {transitions}; audio cache: {} bytes in {}", cache.total_bytes(), cache_dir.display());
    if report {
        // End the play (the final ping), then read the history back.
        player.stop();
        tokio::time::sleep(Duration::from_secs(6)).await;
        let h = svc.history(&svc.context()).await.map_err(|e| anyhow!("{}", e.message))?;
        let first = h.groups.first();
        let yes = first.is_some_and(|g| g.rows.iter().take(3).any(|r| r.track.id == target));
        println!("history after: {target} near the top of \"{}\": {}{}", first.map(|g| g.title.as_str()).unwrap_or("-"), if yes { "yes" } else { "no" }, if was_there { " (it was already there before)" } else { "" });
    }
    Ok(())
}

/// The app's downloads (its folder and index), with nothing started: the
/// caller starts the worker when it means to download.
async fn cli_downloads(svc: Arc<YtMusic>, resolver: Arc<encore_resolver::YtDlpResolver>, cache: Option<encore_audio::AudioCache>) -> Result<(Arc<encore_downloads::DownloadManager>, Arc<encore_core::store::Store>)> {
    // Never a temporary folder: downloads are kept for good.
    let data = dirs::data_local_dir().ok_or_else(|| anyhow!("this system has no data folder for Encore's downloads"))?.join("app.encore.desktop");
    let store = Arc::new(encore_core::store::Store::open(&data.join("encore.sqlite")).map_err(|e| anyhow!("{}", e.message))?);
    let it = svc.it.clone();
    let deps = encore_downloads::Deps {
        store: store.clone(),
        service: svc.clone(),
        resolver,
        auth: Arc::new(move || encore_cli_auth(&it)),
        gate: Arc::new(encore_core::service::PlaybackGate::default()),
        fetcher: Arc::new(encore_downloads::HttpFetcher::new()),
        cache,
        lyrics: Some(Arc::new(encore_lyrics::LyricsService::new(svc.clone(), Some(store.clone())))),
        art: None,
        loudness: Some(svc.clone()),
        spacing: (Duration::from_secs(2), Duration::from_secs(4)),
    };
    let dl = encore_downloads::DownloadManager::open(data.join("downloads"), deps).map_err(|e| anyhow!("{}", e.message))?;
    Ok((dl, store))
}

fn size(b: f64) -> String {
    if b >= 1e9 { format!("{:.2} GB", b / 1e9) } else { format!("{:.1} MB", b / 1e6) }
}

async fn downloads(svc: Arc<YtMusic>, cmd: DownloadsCmd, json: bool) -> Result<()> {
    use encore_downloads::{DownloadState, DownloadsUpdate};
    let fetching = matches!(cmd, DownloadsCmd::Add { .. } | DownloadsCmd::Keep { .. } | DownloadsCmd::Sync { .. } | DownloadsCmd::Resume);
    let resolver = cli_resolver(fetching).await?;
    let cache_dir = dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("app.encore.desktop/audio");
    let cache = encore_audio::AudioCache::new(&cache_dir, encore_audio::cache::DEFAULT_LIMIT).ok();
    let (dl, _) = cli_downloads(svc.clone(), resolver, cache).await?;
    let e = |e: encore_core::AppError| anyhow!("{} ({:?})", e.message, e.code);
    let ids = |v: &[String]| v.iter().map(|x| ItemId::parse(x)).collect::<Vec<_>>();
    match cmd {
        DownloadsCmd::List => {
            let snap = dl.snapshot();
            out(json, &snap, |s| {
                println!("{} songs, {} ({}){}", s.summary.songs, size(s.summary.bytes), dl.dir().display(), s.summary.message.as_ref().map(|m| format!(". {m}")).unwrap_or_default());
                for i in &s.items {
                    let fmt = match (&i.codec, i.kbps) {
                        (Some(c), Some(k)) => format!("{c} {k} kbps"),
                        (Some(c), None) => c.clone(),
                        _ => String::new(),
                    };
                    println!("  {:<11} {}  {}  {}{}", format!("{:?}", i.state).to_lowercase(), show::track(&i.track), size(i.bytes), fmt, i.error.as_ref().map(|x| format!("  ({x})")).unwrap_or_default());
                }
                for c in &s.collections {
                    let who = c.channel.as_deref().map(|n| format!(", kept by {n}")).unwrap_or_default();
                    println!("kept: {}  {}  {}/{} songs{}{}", c.card.id, c.card.title, c.ready, c.songs, who, c.error.as_ref().map(|x| format!("  ({x})")).unwrap_or_default());
                }
            });
            return Ok(());
        }
        DownloadsCmd::Add { videos } => {
            let svc2 = svc.clone();
            let ctx = svc2.context();
            let mut tracks = Vec::new();
            for id in ids(&videos) {
                tracks.push(svc2.track(&ctx, &id).await.map_err(e)?);
            }
            let n = dl.add(tracks).map_err(e)?;
            println!("{n} waiting");
        }
        DownloadsCmd::Keep { id } => {
            let c = svc.collection(&svc.context(), &ItemId::parse(&id)).await.map_err(e)?;
            println!("keeping {} ({})", c.title, c.id);
            dl.keep(c, None).map_err(e)?;
        }
        DownloadsCmd::Unkeep { id } => {
            let r = dl.unkeep(&ItemId::parse(&id)).map_err(e)?;
            println!("no longer kept; {} songs removed", r.removed);
            return Ok(());
        }
        DownloadsCmd::Remove { videos } => {
            let r = dl.remove(&ids(&videos)).map_err(e)?;
            println!("{} removed", r.removed);
            for k in &r.kept {
                println!("  {} stays: \"{}\" keeps it", k.track_id, k.collection);
            }
            return Ok(());
        }
        DownloadsCmd::Sync { id } => dl.sync(id.map(|x| ItemId::parse(&x)).as_ref()),
        DownloadsCmd::Pause => {
            dl.pause(true);
            println!("paused");
            return Ok(());
        }
        DownloadsCmd::Resume => {
            dl.pause(false);
            dl.retry();
        }
    }
    // Fetch in the foreground until the queue is done, printing each song.
    let mut rx = dl.subscribe();
    tokio::spawn(async move {
        while let Ok(u) = rx.recv().await {
            if let DownloadsUpdate::Item { item, .. } = u {
                let what = match item.state {
                    DownloadState::Downloading => "downloading".to_string(),
                    DownloadState::Ready => format!("done, {}", size(item.bytes)),
                    DownloadState::Failed => format!("failed: {}", item.error.unwrap_or_default()),
                    DownloadState::Queued => match item.error {
                        Some(x) => format!("will try again: {x}"),
                        None => continue,
                    },
                    DownloadState::Removed => "removed".to_string(),
                };
                println!("{} — {}: {what}", item.track.title, item.track.artist_line());
            }
        }
    });
    dl.start();
    let t0 = Instant::now();
    let wait = dl.settle().await;
    let s = dl.summary();
    println!("{} songs downloaded, {} waiting, {} failed ({:.0}s)", s.songs, s.waiting, s.failed, t0.elapsed().as_secs_f64());
    if wait.is_some() {
        if let Some(m) = s.message {
            println!("{m}");
        }
    }
    Ok(())
}

/// Cookies for the resolver from the signed-in session, if any.
/// The resolver on the app's own component folder. With `install`, missing
/// components are downloaded first (running the CLI is the approval).
async fn cli_resolver(install: bool) -> Result<Arc<encore_resolver::YtDlpResolver>> {
    let cfg = encore_resolver::ResolverConfig::default_paths();
    let rt = encore_resolver::Runtimes::new(cfg.runtimes());
    let r = encore_resolver::YtDlpResolver::new(cfg, rt.clone());
    if install && !rt.ready() {
        install_runtimes(&rt).await?;
    }
    Ok(r)
}

async fn install_runtimes(rt: &Arc<encore_resolver::Runtimes>) -> Result<()> {
    let st = rt.status();
    eprintln!("installing player components into {} ({:.1} MB to download)", st.location, st.download_bytes / 1e6);
    let last = Arc::new(parking_lot::Mutex::new((None, -1i64)));
    rt.on_change(move |s| {
        let Some(p) = &s.progress else { return };
        let pct = (p.fraction * 100.0) as i64;
        let mut l = last.lock();
        if l.0 != Some(p.phase) || pct >= l.1 + 10 {
            *l = (Some(p.phase), pct);
            eprintln!("  {:?} {}% {}", p.phase, pct, p.component.as_deref().unwrap_or(""));
        }
    });
    rt.approve().map_err(|e| anyhow!("{}", e.message))?;
    rt.install().await.map_err(|e| anyhow!("{} ({:?})", e.message, e.kind))
}

fn encore_cli_auth(it: &InnerTube) -> Option<encore_core::service::ResolveAuth> {
    let s = it.session()?;
    let ch = s.channel.as_ref()?;
    Some(encore_core::service::ResolveAuth { cookies: s.jar.records().to_vec(), auth_user: ch.auth_user, page_id: ch.page_id.clone() })
}

fn play_file(path: PathBuf, seek: f64, max: Option<f64>) -> Result<()> {
    if !path.is_file() {
        bail!("no such file: {}", path.display());
    }
    let (codec, rate, dur) = encore_audio::decode::probe_info(&SourceSpec::File(path.clone())).map_err(anyhow::Error::msg)?;
    println!("file: codec={codec} rate={rate} duration={}", dur.map(|d| format!("{d:.2}s")).unwrap_or("?".into()));
    let engine = Engine::new(None).map_err(anyhow::Error::msg)?;
    let events = engine.take_events().unwrap();
    engine.load(1, SourceSpec::File(path), seek, 0.0, true);
    let t0 = Instant::now();
    let mut last = Instant::now();
    loop {
        while let Ok(ev) = events.try_recv() {
            match &ev {
                EngineEvent::Ended { .. } => {
                    report(&engine, "ended");
                    return Ok(());
                }
                EngineEvent::Failed { error, .. } => bail!("playback failed: {error}"),
                other => println!("event: {other:?}"),
            }
        }
        if last.elapsed() > Duration::from_millis(500) {
            last = Instant::now();
            report(&engine, "playing");
        }
        if max.is_some_and(|m| t0.elapsed().as_secs_f64() > m) {
            report(&engine, "stopped (max)");
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn report(engine: &Engine, what: &str) {
    let s = engine.status();
    println!(
        "{what}: pos={:.2}s dur={} frames_out={} callbacks={} underruns={} device=\"{}\" {}Hz/{}ch",
        s.position,
        s.duration.map(|d| format!("{d:.2}s")).unwrap_or("?".into()),
        s.frames_out,
        s.callbacks,
        s.underruns,
        s.device,
        s.sample_rate,
        s.channels
    );
}
