<script lang="ts">
  import { call, type Collection, type Entry } from "$lib/backend";
  import { catalog } from "$lib/catalog.svelte";
  import { kindLabel, totalDuration } from "$lib/data";
  import { Resource } from "$lib/load.svelte";
  import { account, collectionMenu, downloads, editPlaylist, keptNote, nav, player, playlists, ui } from "$lib/state.svelte";
  import Cover from "$lib/components/Cover.svelte";
  import KeepButton from "$lib/components/KeepButton.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import Shelves from "$lib/components/Shelves.svelte";
  import Status from "$lib/components/Status.svelte";
  import TrackTable from "$lib/components/TrackTable.svelte";

  let { id }: { id: string } = $props();

  let entries = $state<Entry[]>([]);
  let cursor = $state<string | null>(null);
  let loadingMore = $state(false);
  const res = new Resource<Collection>(async () => {
    const c = await call((a) => a.collection(id));
    catalog.addCollection(c);
    entries = c.entries;
    cursor = c.cursor;
    return c;
  });
  const c = $derived(res.data);

  // Load the rest of a long playlist in the background, in order.
  $effect(() => {
    if (cursor && !loadingMore) void more();
  });
  async function more() {
    if (!cursor || loadingMore || entries.length > 5000) return;
    loadingMore = true;
    try {
      const p = await call((a) => a.collectionMore(cursor!));
      catalog.addTracks(p.entries.map((e) => e.track));
      entries = [...entries, ...p.entries];
      cursor = p.cursor;
    } catch {
      cursor = null;
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
  // Offline, only downloaded songs play: the header says so (D23).
  const here = $derived(tracks.filter((t) => downloads.has(t.id)).length);
  const partial = $derived(!ui.online && here < tracks.length);
  const note = $derived(keptNote(downloads.keptFor(id), downloads.syncing(id)));

  // Offline and not kept, the header plays the downloaded songs it shows.
  function playHere(shuffle: boolean) {
    if (ui.online || downloads.keptFor(id)) return player.playCollection(id, 0, shuffle);
    const local = tracks.filter((t) => downloads.has(t.id));
    player.playTracks(local, shuffle ? Math.floor(Math.random() * local.length) : 0, { label: c?.title ?? "Downloads", collectionId: id, radioPlaylist: null }, shuffle);
  }
  // Offline, edits wait (D23): the controls go until the connection is back.
  const mine = $derived(!!c?.editable && account.signedIn && !!c?.playlistId && ui.online);
  const privacyLabel = { private: "Private", unlisted: "Unlisted", public: "Public" } as const;
  const privacyIcon = { private: "lock", unlisted: "link", public: "globe" } as const;
  // Reorder only a fully loaded list of rows that have entry ids (D10).
  const canReorder = $derived(mine && !cursor && !loadingMore && entries.length > 1 && entries.every((e) => !!e.setVideoId));
  let moving = $state(false);

  /** The row at `from` takes the place of the row at `to`. */
  async function move(from: number, to: number) {
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

  const moveMenu = (i: number) =>
    canReorder
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
    const e = entries[i];
    if (!c?.playlistId || !e) return;
    const before = entries;
    entries = entries.filter((_, k) => k !== i);
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
      <TrackTable {tracks} label={c.title} collectionId={id} showAlbum={!isAlbum} showArt={!isAlbum} showPlays={isAlbum} onremove={c.editable && account.signedIn && ui.online ? remove : undefined} endMenu={moveMenu} reorder={canReorder ? move : undefined} />
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
</style>
