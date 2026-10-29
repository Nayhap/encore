<script lang="ts">
  import type { Track } from "$lib/backend";
  import { artistLine, fmt } from "$lib/data";
  import { nav, player, trackMenu, ui, type MenuItem } from "$lib/state.svelte";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";

  let {
    tracks,
    label,
    collectionId,
    showAlbum = true,
    showArt = true,
    showPlays = false,
    head = true,
    onremove,
    extraMenu,
    endMenu,
    reorder,
  }: {
    tracks: Track[];
    label: string;
    collectionId?: string;
    showAlbum?: boolean;
    showArt?: boolean;
    showPlays?: boolean;
    /** Show the column header row. */
    head?: boolean;
    onremove?: (index: number) => void;
    /** Row-specific actions, listed first in the row's menu. */
    extraMenu?: (index: number) => MenuItem[];
    /** Row-specific actions, listed last. */
    endMenu?: (index: number) => MenuItem[];
    /** Drag rows to reorder: the row at `from` takes the place of `to`. */
    reorder?: (from: number, to: number) => void;
  } = $props();

  const menuFor = (i: number) => {
    const items = trackMenu(tracks[i].id);
    const extra = extraMenu?.(i) ?? [];
    const rm = onremove ? [{ label: "Remove from this playlist", icon: "x", run: () => onremove(i) }] : [];
    return [...extra, ...rm, ...items, ...(endMenu?.(i) ?? [])];
  };

  let dragFrom = $state<number | null>(null);
  let dragOver = $state<number | null>(null);
  function drop(k: number) {
    if (reorder && dragFrom !== null && dragFrom !== k) reorder(dragFrom, k);
    dragFrom = dragOver = null;
  }

  const play = (i: number) => {
    const t = tracks[i];
    if (player.current?.id === t.id && player.source?.label === label) return player.toggle();
    if (!t.playable) return ui.say(`“${t.title}” is not available to play`);
    player.playTracks(tracks.filter((x) => x.playable), tracks.slice(0, i).filter((x) => x.playable).length, { label, collectionId: collectionId ?? null, radioPlaylist: null });
  };

  function rowKey(e: KeyboardEvent, i: number) {
    if (e.key === "Enter") {
      e.preventDefault();
      play(i);
    }
  }
</script>

<div class="table" class:no-album={!showAlbum} class:plays={showPlays} role="table" aria-label="{label} songs">
  {#if head}<div class="row head" role="row">
    <span role="columnheader">#</span>
    <span role="columnheader">Title</span>
    {#if showAlbum}<span role="columnheader">Album</span>{/if}
    {#if showPlays}<span role="columnheader" class="r">Plays</span>{/if}
    <span role="columnheader" class="r"><Icon name="clock" size={15} /></span>
    <span></span>
  </div>{/if}
  {#each tracks as t, i (t.id + ":" + i)}
    {@const id = t.id}
    {@const current = player.current?.id === id}
    {@const artist = t.artists.find((a) => a.id)}
    <div
      class="row"
      class:current
      class:unplayable={!t.playable}
      class:over={reorder && dragOver === i && dragFrom !== null && dragFrom !== i}
      class:below={reorder && dragOver === i && dragFrom !== null && dragFrom < i}
      class:dragging={dragFrom === i}
      draggable={reorder ? "true" : undefined}
      ondragstart={reorder ? (e) => { dragFrom = i; e.dataTransfer!.effectAllowed = "move"; } : undefined}
      ondragover={reorder ? (e) => { e.preventDefault(); dragOver = i; } : undefined}
      ondragend={reorder ? () => (dragFrom = dragOver = null) : undefined}
      ondrop={reorder ? () => drop(i) : undefined}
      role="row"
      tabindex="0"
      ondblclick={() => play(i)}
      onkeydown={(e) => rowKey(e, i)}
      oncontextmenu={(e) => ui.openMenu(e, menuFor(i))}
    >
      <span class="n" role="cell">
        {#if current && player.playing}
          <span class="eq" aria-hidden="true"><i></i><i></i><i></i></span>
        {:else}
          <span class="mono num">{i + 1}</span>
        {/if}
        <button class="rowplay" aria-label={current && player.playing ? "Pause" : `Play ${t.title}`} onclick={() => play(i)}>
          <Icon name={current && player.playing ? "pause" : "play"} size={15} />
        </button>
      </span>
      <span class="title" role="cell">
        {#if showArt}<span class="thumb"><Cover art={t.art} radius={4} size={120} label={t.title} /></span>{/if}
        <span class="txt">
          <span class="t">
            <span class="ell">{t.title}</span>
            {#if t.explicit}<span class="e" title="Explicit">E</span>{/if}
          </span>
          {#if artist?.id}
            <button class="a ell" onclick={() => nav.go({ name: "artist", id: artist.id! })}>{artistLine(t)}</button>
          {:else}
            <span class="a ell">{artistLine(t)}</span>
          {/if}
        </span>
      </span>
      {#if showAlbum}
        <span role="cell" class="album">
          {#if t.album?.id}
            <button class="ell" onclick={() => nav.go({ name: "collection", id: t.album!.id! })}>{t.album.name}</button>
          {:else}
            <span class="ell dim">{t.album?.name ?? ""}</span>
          {/if}
        </span>
      {/if}
      {#if showPlays}<span role="cell" class="r mono dim">{t.plays?.replace(/ plays?$/, "") ?? "—"}</span>{/if}
      <span role="cell" class="r tail">
        <button
          class="icon-btn like"
          class:on={player.liked.has(id)}
          class:keep={player.liked.has(id)}
          aria-pressed={player.liked.has(id)}
          aria-label={player.liked.has(id) ? "Remove from liked music" : "Add to liked music"}
          onclick={() => player.toggleLike(id)}
        ><Icon name={player.liked.has(id) ? "heartFill" : "heart"} size={16} /></button>
        <span class="mono dim">{fmt(t.duration)}</span>
      </span>
      <span role="cell">
        <button class="icon-btn more" aria-label="More actions for {t.title}" onclick={(e) => ui.openMenu(e, menuFor(i))}><Icon name="more" size={17} /></button>
      </span>
    </div>
  {/each}
</div>

<style>
  .row.unplayable {
    opacity: 0.45;
  }
  .row {
    position: relative;
  }
  .row.over::before {
    content: "";
    position: absolute;
    left: 8px;
    right: 8px;
    top: -1px;
    height: 2px;
    border-radius: 2px;
    background: var(--accent);
  }
  .row.over.below::before {
    top: auto;
    bottom: -1px;
  }
  .row.dragging {
    opacity: 0.4;
  }
  .table {
    --cols: 40px minmax(0, 1.6fr) minmax(0, 1fr) 96px 40px;
  }
  .table.no-album {
    --cols: 40px minmax(0, 1fr) 96px 40px;
  }
  .table.plays {
    --cols: 40px minmax(0, 1.6fr) minmax(0, 1fr) 80px 96px 40px;
  }
  .table.plays.no-album {
    --cols: 40px minmax(0, 1fr) 80px 96px 40px;
  }
  .row {
    display: grid;
    grid-template-columns: var(--cols);
    align-items: center;
    gap: 12px;
    min-height: 56px;
    padding: 0 8px;
    border-radius: 8px;
  }
  .row.head {
    min-height: 34px;
    font-size: 12px;
    font-weight: 600;
    color: var(--dim);
    border-bottom: 1px solid var(--line);
    border-radius: 0;
    margin-bottom: 6px;
  }
  .row.head :global(svg) {
    margin-left: auto;
  }
  .row:not(.head):hover,
  .row:not(.head):focus-within {
    background: var(--hover);
  }
  .row:focus-visible {
    outline-offset: -2px;
  }
  .r {
    text-align: right;
    justify-content: flex-end;
  }
  .dim {
    color: var(--mute);
    font-size: 12.5px;
  }
  .n {
    position: relative;
    display: grid;
    place-items: center;
    height: 32px;
    color: var(--mute);
  }
  .num {
    font-size: 13px;
  }
  .rowplay {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    opacity: 0;
    color: var(--text);
  }
  .row:hover .rowplay,
  .rowplay:focus-visible {
    opacity: 1;
  }
  .row:hover .num,
  .row:hover .eq {
    opacity: 0;
  }
  .title {
    display: flex;
    align-items: center;
    gap: 12px;
    min-width: 0;
  }
  .thumb {
    width: 40px;
    flex: none;
  }
  .txt {
    display: grid;
    min-width: 0;
  }
  .t {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    font-weight: 550;
  }
  .current .t {
    color: var(--accent);
  }
  .e {
    flex: none;
    display: grid;
    place-items: center;
    width: 15px;
    height: 15px;
    border-radius: 3px;
    background: color-mix(in srgb, var(--text) 22%, transparent);
    color: var(--ink);
    font-size: 9.5px;
    font-weight: 800;
  }
  .a {
    font-size: 13px;
    color: var(--mute);
    text-align: left;
    justify-self: start;
    max-width: 100%;
  }
  .album {
    min-width: 0;
    font-size: 13px;
    color: var(--mute);
  }
  .album button {
    max-width: 100%;
    display: block;
  }
  .a:hover,
  .album button:hover {
    color: var(--text);
    text-decoration: underline;
    text-underline-offset: 3px;
  }
  .tail {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .like {
    width: 28px;
    height: 28px;
    opacity: 0;
  }
  .like.keep,
  .row:hover .like,
  .like:focus-visible {
    opacity: 1;
  }
  .more {
    opacity: 0;
  }
  .row:hover .more,
  .more:focus-visible {
    opacity: 1;
  }
  .eq {
    display: flex;
    align-items: flex-end;
    gap: 2px;
    height: 13px;
    color: var(--accent);
  }
  .eq i {
    width: 3px;
    background: currentColor;
    border-radius: 1px;
    animation: eq 0.9s ease-in-out infinite alternate;
  }
  .eq i:nth-child(2) {
    animation-delay: -0.3s;
  }
  .eq i:nth-child(3) {
    animation-delay: -0.6s;
  }
  @keyframes eq {
    from {
      height: 25%;
    }
    to {
      height: 100%;
    }
  }

  @media (max-width: 1100px) {
    .table:not(.no-album) {
      --cols: 40px minmax(0, 1fr) 96px 40px;
    }
    .table.plays:not(.no-album) {
      --cols: 40px minmax(0, 1fr) 80px 96px 40px;
    }
    .table:not(.no-album) .album,
    .table:not(.no-album) .head > :nth-child(3) {
      display: none;
    }
  }
</style>
