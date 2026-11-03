<script lang="ts">
  import { call, type Collection, type Entry } from "$lib/backend";
  import { catalog } from "$lib/catalog.svelte";
  import { kindLabel, totalDuration } from "$lib/data";
  import { Resource } from "$lib/load.svelte";
  import { account, albumYears, collectionMenu, downloads, editPlaylist, keptNote, nav, player, playlists, stopYears, ui, yearLookup } from "$lib/state.svelte";
  import Cover from "$lib/components/Cover.svelte";
  import KeepButton from "$lib/components/KeepButton.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import Shelves from "$lib/components/Shelves.svelte";
  import Status from "$lib/components/Status.svelte";
  import TrackTable from "$lib/components/TrackTable.svelte";
  import SelectionBar from "$lib/components/library-tools/SelectionBar.svelte";
  import Toolbar from "$lib/components/library-tools/Toolbar.svelte";
  import { ListTools } from "$lib/components/library-tools/tools.svelte";

  let { id, params = null }: { id: string; params?: string | null } = $props();

  let entries = $state<Entry[]>([]);
  let cursor = $state<string | null>(null);
  let loadingMore = $state(false);
  /** The service said the list ended: every row is here (D10). */
  let complete = $state(false);
  const res = new Resource<Collection>(async () => {
    const c = await call((a) => a.collection(id, params));
    catalog.addCollection(c);
    entries = c.entries;
    cursor = c.cursor;
    complete = !c.cursor;
    return c;
  });
  const c = $derived(res.data);

  // Load a long playlist in order, a page at a time: each one waits its
  // turn behind playback and downloads in Rust (D10, D23). The first
  // `AUTO` rows come by themselves; after that a page comes when the end
  // of the list scrolls into view, or all of them on "Load all" (D34).
  const AUTO = 500;
  const MOST = 20000;
  let loadAll = $state(false);
  let nearEnd = $state(false);
  $effect(() => {
    if (cursor && !loadingMore && (entries.length < AUTO || loadAll || nearEnd)) void more();
  });
  function watchEnd(el: HTMLElement) {
    const io = new IntersectionObserver(([e]) => (nearEnd = e.isIntersecting), { rootMargin: "600px 0px" });
    io.observe(el);
    return { destroy: () => io.disconnect() };
  }
  async function more() {
    if (!cursor || loadingMore || entries.length >= MOST) return;
    loadingMore = true;
    try {
      const p = await call((a) => a.collectionMore(cursor!));
      catalog.addTracks(p.entries.map((e) => e.track));
      entries = [...entries, ...p.entries];
      cursor = p.cursor;
      complete = p.complete;
    } catch {
      cursor = null;
      complete = false;
    } finally {
      loadingMore = false;
    }
  }

  // Another surface changed this playlist (or we did): show the server's copy.
  let seenRev = playlists.rev;
  $effect(() => {
    const r = playlists.rev;
    if (r === seenRev) return;
    seenRev = r;
    if (!playlists.last || playlists.last === id) void res.reload();
  });

  const tracks = $derived(entries.map((e) => e.track));

  // Find, sort and select (D34): a view over the rows, never a change to them.
  const tools = new ListTools(() => entries, {
    get noun() {
      return c?.kind === "liked" ? "liked music" : isAlbum ? (c?.kind === "single" ? "single" : c?.kind === "ep" ? "EP" : "album") : "playlist";
    },
    get album() {
      return isAlbum;
    },
  });
  const view = $derived(tools.view);
  const shown = $derived(view.map((e) => e.track));
  $effect(() => {
    if (entries.length) tools.refreshFacts();
  });
  $effect(() => () => stopYears());
  const total = $derived(Math.max(c?.trackCount ?? 0, entries.length));
  const songs = (n: number) => (n === 1 ? "1 song" : `${n.toLocaleString()} songs`);
  // Offline, only downloaded songs play: the header says so (D23).
  const here = $derived(tracks.filter((t) => downloads.has(t.id)).length);
  const partial = $derived(!ui.online && here < tracks.length);
  const note = $derived(keptNote(downloads.keptFor(id), downloads.syncing(id)));

  // Offline and not kept, the header plays the downloaded songs it shows.
  function playHere(shuffle: boolean) {
    // Sorted or found: the songs as shown, in that order.
    if (tools.changed) {
      const ok = shown.filter((t) => downloads.playable(t));
      if (!ok.length) return ui.say("None of these songs can play right now");
      return player.playTracks(ok, shuffle ? Math.floor(Math.random() * ok.length) : 0, { label: c?.title ?? "Playlist", collectionId: id, radioPlaylist: null }, shuffle);
    }
    if (ui.online || downloads.keptFor(id)) return player.playCollection(id, 0, shuffle, params);
    const local = tracks.filter((t) => downloads.has(t.id));
    player.playTracks(local, shuffle ? Math.floor(Math.random() * local.length) : 0, { label: c?.title ?? "Downloads", collectionId: id, radioPlaylist: null }, shuffle);
  }
  // Offline, edits wait (D23): the controls go until the connection is back.
  const mine = $derived(!!c?.editable && account.signedIn && !!c?.playlistId && ui.online);
  const privacyLabel = { private: "Private", unlisted: "Unlisted", public: "Public" } as const;
  const privacyIcon = { private: "lock", unlisted: "link", public: "globe" } as const;
  // Reorder only a list known to be whole, of rows that have entry ids (D10).
  const canReorder = $derived(mine && complete && !loadingMore && entries.length > 1 && entries.every((e) => !!e.setVideoId));
  let moving = $state(false);

  /** The row at `from` takes the place of the row at `to` (in playlist order). */
  async function move(from: number, to: number) {
    if (tools.changed) return;
    const e = entries[from];
    if (!canReorder || moving || !e?.setVideoId || from === to) return;
    const rest = entries.filter((_, i) => i !== from);
    const before = rest[to] ?? null;
    const next = [...rest];
    next.splice(before ? rest.indexOf(before) : rest.length, 0, e);
    const old = entries;
    entries = next;
    moving = true;
    try {
      const r = await call((a) => a.playlistMove(c!.playlistId!, e.setVideoId!, before?.setVideoId ?? null));
      if (r === "rejected") {
        entries = old;
        ui.say("YouTube Music did not accept that order");
      } else if (r === "outcomeUnknown") {
        ui.say("Not sure that worked; reloading the playlist");
        void res.reload();
      }
    } catch (err: any) {
      entries = old;
      ui.say(`Could not move: ${err.message}`);
    } finally {
      moving = false;
    }
  }

  const keepOrder = "Return to playlist order to reorder";
  const moveMenu = (i: number) =>
    mine && tools.changed && !tools.selecting
      ? [{ label: "Move up or down", icon: "grip", disabled: keepOrder, run: () => {} }]
      : canReorder && !tools.changed
      ? [
          ...(i > 0 ? [{ label: "Move up", icon: "up", run: () => move(i, i - 1) }] : []),
          ...(i < entries.length - 1 ? [{ label: "Move down", icon: "down", run: () => move(i, i + 1) }] : []),
        ]
      : [];
  const isAlbum = $derived(c ? c.kind === "album" || c.kind === "single" || c.kind === "ep" : false);
  const cur = $derived(player.source?.collectionId === id);
  const hue = $derived(c ? catalog.hue(c.art, c.id)[0] : 210);
  let saved = $state<boolean | null>(null);
  $effect(() => {
    saved = c?.saved ?? null;
  });
  const artist = $derived(c?.artists.find((a) => a.id));

  async function remove(i: number) {
    const e = view[i];
    if (!c?.playlistId || !e) return;
    const before = entries;
    entries = entries.filter((x) => x !== e);
    try {
      const r = await call((a) => a.playlistRemove(c.playlistId!, [e]));
      if (r === "rejected") {
        entries = before;
        ui.say("YouTube Music did not accept that change");
      } else if (r === "outcomeUnknown") {
        ui.say("Not sure that worked; reloading the playlist");
        void res.reload();
      } else ui.say(`Removed “${e.track.title}”`);
    } catch (err: any) {
      entries = before;
      ui.say(`Could not remove: ${err.message}`);
    }
  }

  /** Remove the selected rows, in one write (D20). */
  function removeSelected() {
    const rows = tools.chosen.filter((e) => e.setVideoId);
    if (!c?.playlistId || !rows.length) return ui.say("These songs can't be removed from this playlist");
    const dupes = tools.reviewing;
    // Reviewing, say plainly when more than extra copies would go.
    const { firsts, versions } = tools.beyondCopies;
    const copies = rows.length - firsts - versions;
    const besides = [firsts && (firsts === 1 ? "1 original" : `${firsts} originals`), versions && (versions === 1 ? "1 other version" : `${versions} other versions`)].filter(Boolean).join(" and ");
    ui.confirm = !dupes
      ? { title: `Remove ${songs(rows.length)}?`, text: `They're removed from “${c.title}” on YouTube Music.`, confirm: "Remove", danger: true, run: () => void removeRows(rows, false) }
      : besides
        ? {
            title: `Remove ${songs(rows.length)}, including ${besides}?`,
            text: `${firsts ? `The originals are the first row of their song: removing them can take a song out of “${c.title}” altogether.` : `Each song stays in “${c.title}” as the version that isn't selected.`}${copies ? ` The other ${copies === 1 ? "one is an extra copy" : `${copies} are extra copies`}.` : ""}`,
            confirm: "Remove",
            danger: true,
            run: () => void removeRows(rows, true),
          }
        : { title: `Remove ${rows.length === 1 ? "1 extra copy" : `${rows.length} extra copies`}?`, text: `The first copy of each song stays in “${c.title}”.`, confirm: "Remove copies", danger: true, run: () => void removeRows(rows, true) };
  }
  async function removeRows(rows: Entry[], dupes: boolean) {
    const gone = new Set(rows.map((e) => e.entryId));
    const before = entries;
    entries = entries.filter((e) => !gone.has(e.entryId));
    tools.done();
    try {
      const r = await call((a) => a.playlistRemove(c!.playlistId!, rows));
      if (r === "rejected") {
        entries = before;
        ui.say("YouTube Music did not accept that change");
      } else if (r === "outcomeUnknown") {
        ui.say("Not sure that worked; reloading the playlist");
        void res.reload();
      } else ui.say(`Removed ${songs(rows.length)}`);
    } catch (err: any) {
      entries = before;
      ui.say(`Could not remove: ${err.message}`);
    }
  }

  // What the table shows for the current sort, beside the duration.
  const metric = $derived.by(() => {
    const k = tools.key;
    const f = tools.facts;
    if (k === "year") return { label: "Year", value: (i: number) => { const a = view[i]?.track.album?.id; const y = a ? albumYears.get(a) : null; return y ? String(y) : null; } };
    if (k === "plays") return { label: "Your plays", value: (i: number) => f.plays[view[i]?.track.id]?.toLocaleString() ?? null };
    if (k === "lastfm") return { label: "Last.fm", value: (i: number) => f.lastfm?.[view[i]?.track.id]?.toLocaleString() ?? null };
    if (k === "bpm") return { label: "BPM", value: (i: number) => { const b = f.bpm[view[i]?.track.id]; return b ? String(Math.round(b)) : null; } };
    return undefined;
  });
  const select = $derived(tools.selecting ? { has: (i: number) => tools.selected.has(view[i]?.entryId), toggle: (i: number, range: boolean) => view[i] && tools.toggle(view[i], range) } : undefined);
  const tag = $derived(
    tools.reviewing
      ? (i: number) => {
          const e = view[i];
          const k = e && tools.dups.kind.get(e.entryId);
          return k === "copy" ? { text: "Copy", strong: true } : k === "version" ? { text: e.track.kind === "video" ? "Video" : "Song" } : { text: "Original" };
        }
      : undefined,
  );
  // Reviewing, each original starts a group with its copies and versions.
  const groupStart = $derived(tools.reviewing ? (i: number) => i > 0 && !!view[i] && !tools.dups.kind.has(view[i].entryId) : undefined);
  const years = $derived(tools.key === "year" && yearLookup.total > 0 && yearLookup.done < yearLookup.total);

  async function toggleSave() {
    if (!c?.playlistId) return;
    if (!account.signedIn) return ui.say("Sign in to save to your library");
    const want = !saved;
    saved = want;
    try {
      const r = await call((a) => a.setSaved(c.playlistId!, want));
      if (r === "rejected") {
        saved = !want;
        ui.say("YouTube Music did not accept that change");
      } else ui.say(want ? "Saved to library" : "Removed from library");
    } catch (e: any) {
      saved = !want;
      ui.say(`Could not change the library: ${e.message}`);
    }
  }
</script>

<div class="page">
  <Status loading={res.loading && !c} error={res.error} onretry={() => res.reload()} />
  {#if c}
    <header class="hero" style:--ch={hue}>
      <div class="art"><Cover art={c.art} radius={12} label={c.title} /></div>
      <div class="info">
        <p class="kind">{kindLabel[c.kind]}</p>
        <h1 class:long={c.title.length > 22}>{c.title}</h1>
        <p class="meta">
          {#if artist?.id}
            <button class="by" onclick={() => nav.go({ name: "artist", id: artist.id! })}>{c.artists.map((a) => a.name).join(", ")}</button>
          {:else if c.artists.length}
            <span class="by">{c.artists.map((a) => a.name).join(", ")}</span>
          {/if}
          {#if c.year}<span>{c.year}</span>{/if}
          <span>{c.trackCount ?? entries.length} {(c.trackCount ?? entries.length) === 1 ? "song" : "songs"}{loadingMore ? " (loading…)" : ""}</span>
          {#if tracks.length}<span>{totalDuration(tracks)}</span>{/if}
          {#if mine && c.privacy}<span class="priv"><Icon name={privacyIcon[c.privacy]} size={13} />{privacyLabel[c.privacy]}</span>{/if}
        </p>
        {#if c.description}<p class="desc">{c.description}</p>{/if}
        <div class="actions">
          <button class="pill primary" disabled={!tracks.length || (partial && !here)} onclick={() => (cur ? player.toggle() : playHere(false))}>
            <Icon name={cur && player.playing ? "pause" : "play"} size={16} />
            {cur && player.playing ? "Pause" : partial ? "Play downloaded" : "Play"}
          </button>
          <button class="pill" disabled={!tracks.length || (partial && !here)} onclick={() => playHere(true)}><Icon name="shuffle" size={16} />{partial ? "Shuffle downloaded" : "Shuffle"}</button>
          {#if tracks.length}<KeepButton {id} />{/if}
          {#if c.kind !== "liked" && !c.editable && c.playlistId}
            <button
              class="icon-btn big"
              class:on={!!saved}
              aria-pressed={!!saved}
              aria-label={saved ? "Remove from library" : "Save to library"}
              title={saved ? "Remove from library" : "Save to library"}
              disabled={!ui.online}
              onclick={toggleSave}
            ><Icon name={saved ? "check" : "plus"} size={20} /></button>
          {/if}
          {#if mine}
            <button class="icon-btn big" aria-label="Edit details" title="Edit details" onclick={() => editPlaylist(id)}><Icon name="edit" size={18} /></button>
          {/if}
          <button class="icon-btn big" aria-label="More actions" onclick={(e) => ui.openMenu(e, collectionMenu(id))}><Icon name="more" size={19} /></button>
        </div>
        {#if note}<p class="kept-note">{note}</p>{/if}
        {#if !ui.online && !here && tracks.length}<p class="kept-note">You're offline, and none of these songs are downloaded.</p>{/if}
      </div>
    </header>

    {#if tracks.length}
      {#if tools.selecting}
        <SelectionBar {tools} from={c.playlistId} onremove={mine ? removeSelected : undefined} />
      {:else if entries.length > 1}
        <!-- The list's own toolbar (D34): what the list shows on the left, the tools on the right. -->
        <div class="list-head">
          <Toolbar {tools} duplicates={mine && tools.dupCount > 0} />
          <!-- One line at most, the most useful first; none when the empty state says it. -->
          {#if years}
            <p class="tools-note">Looking up release years · {yearLookup.done} of {yearLookup.total} albums · <button onclick={stopYears}>Stop</button></p>
          {:else if tools.changed && cursor}
            <p class="tools-note">{tools.filtered ? "Searching" : "Sorting"} {entries.length.toLocaleString()} of {total.toLocaleString()} songs · <button onclick={() => (loadAll = true)} disabled={loadAll}>{loadAll ? "Loading all…" : "Load all"}</button></p>
          {:else if tools.query.trim() && view.length}
            <p class="tools-note">{view.length === 1 ? "1 song matches" : `${view.length.toLocaleString()} songs match`}</p>
          {:else if tools.key === "year" && yearLookup.skipped}
            <p class="tools-note">Years of {yearLookup.skipped.toLocaleString()} albums weren't looked up; their songs sort last.</p>
          {/if}
        </div>
      {/if}
      {#if view.length}
        <TrackTable tracks={shown} label={c.title} collectionId={id} showAlbum={!isAlbum} showArt={!isAlbum} showPlays={isAlbum || tools.key === "youtube"} {metric} {select} {tag} {groupStart} onremove={c.editable && account.signedIn && ui.online ? remove : undefined} endMenu={moveMenu} reorder={canReorder && !tools.changed ? move : undefined} />
      {:else}
        <Status empty={cursor ? `No songs match “${tools.query.trim()}” in the ${entries.length.toLocaleString()} loaded so far.` : `No songs match “${tools.query.trim()}”.`} />
      {/if}
      {#if cursor && !tools.changed}
        <p class="more-rows" use:watchEnd>
          {#if loadingMore}Loading songs… {entries.length.toLocaleString()} of {total.toLocaleString()}{:else}{entries.length.toLocaleString()} of {total.toLocaleString()} songs{/if}
          {#if !loadAll} · <button onclick={() => (loadAll = true)}>Load all</button>{/if}
        </p>
      {/if}
    {:else}
      <Status empty="This collection has no songs that can be played here." />
    {/if}
    {#if c.related.length}<Shelves shelves={c.related} />{/if}
  {/if}
</div>

<style>
  .hero {
    position: relative;
    display: flex;
    align-items: flex-end;
    gap: 28px;
    padding: 12px 0 28px;
  }
  .hero::before {
    content: "";
    position: absolute;
    inset: -120px -32px 0;
    background: linear-gradient(hsl(var(--ch) 60% 50% / 0.22), transparent);
    z-index: -1;
    pointer-events: none;
  }
  .art {
    width: clamp(160px, 18vw, 232px);
    flex: none;
    box-shadow: 0 24px 50px -20px rgb(0 0 0 / 0.6);
    border-radius: 12px;
  }
  .info {
    min-width: 0;
    display: grid;
    gap: 8px;
  }
  .kind {
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--mute);
  }
  h1 {
    font-family: var(--font-display);
    font-size: clamp(36px, 4.6vw, 64px);
    font-weight: 700;
    line-height: 1;
    letter-spacing: -0.035em;
    font-variation-settings: "wdth" 88;
    text-wrap: balance;
    padding-bottom: 4px;
  }
  h1.long {
    font-size: clamp(28px, 3.2vw, 44px);
  }
  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: 0 6px;
    font-size: 13.5px;
    color: var(--mute);
  }
  .meta > * + *::before {
    content: "·";
    margin-right: 6px;
  }
  .by {
    color: var(--text);
    font-weight: 600;
  }
  button.by:hover {
    text-decoration: underline;
    text-underline-offset: 3px;
  }
  .priv {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .desc {
    color: var(--mute);
    font-size: 13.5px;
    max-width: 60ch;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 10px;
  }
  .big {
    width: 38px;
    height: 38px;
  }
  .kept-note {
    font-size: 13px;
    color: var(--mute);
  }
  .list-head {
    display: grid;
    gap: 6px;
    margin: -6px 0 10px;
  }
  .tools-note {
    min-width: 0;
    padding: 0 4px;
    font-size: 13px;
    color: var(--mute);
  }
  .tools-note button,
  .more-rows button {
    color: var(--accent);
    font-weight: 600;
  }
  .tools-note button:hover,
  .more-rows button:hover {
    text-decoration: underline;
    text-underline-offset: 3px;
  }
  .more-rows {
    margin: 14px 8px 0;
    font-size: 13px;
    color: var(--mute);
  }
</style>
