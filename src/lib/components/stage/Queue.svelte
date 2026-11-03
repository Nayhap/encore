<script lang="ts">
  // Up next: where the queue came from, its songs, and Autoplay.
  import { artistLine, fmt } from "$lib/data";
  import { downloads, nav, player, trackMenu, ui } from "$lib/state.svelte";
  import Cover from "../Cover.svelte";
  import DownloadMark from "../DownloadMark.svelte";
  import Icon from "../Icon.svelte";
  import Autoplay from "./Autoplay.svelte";

  let dragFrom = $state<number | null>(null);
  let dragOver = $state<number | null>(null);

  const qi = (k: number) => (player.index ?? -1) + 1 + k;

  function drop(k: number) {
    if (dragFrom !== null) player.move(qi(dragFrom), qi(k));
    dragFrom = dragOver = null;
  }
</script>

<section class="queue" aria-label="Up next">
  <header>
    <div>
      <h3>Up next</h3>
      {#if player.source}
        <p class="from">
          Playing from
          {#if player.source.collectionId}
            <button onclick={() => nav.go({ name: "collection", id: player.source!.collectionId! })}>{player.source.label}</button>
          {:else}
            <span>{player.source.label}</span>
          {/if}
        </p>
      {/if}
    </div>
    {#if player.upNext.length}
      <button class="clear" onclick={() => (player.clearUpNext(), ui.say("Cleared up next"))}>Clear</button>
    {/if}
  </header>

  <ol>
    {#each player.upNext as e, k (e.uid)}
      {@const t = e.track}
      {@const id = t.id}
      <li
        class="row"
        class:unavailable={!downloads.playable(t)}
        class:over={dragOver === k && dragFrom !== k}
        class:dragging={dragFrom === k}
        draggable="true"
        ondragstart={(e) => { dragFrom = k; e.dataTransfer!.effectAllowed = "move"; }}
        ondragover={(e) => { e.preventDefault(); dragOver = k; }}
        ondragend={() => (dragFrom = dragOver = null)}
        ondrop={() => drop(k)}
        oncontextmenu={(ev) => ui.openMenu(ev, [{ label: "Remove from queue", icon: "x", run: () => player.removeAt(qi(k)) }, ...trackMenu(id, e.uid)])}
      >
        <span class="grip" aria-hidden="true"><Icon name="grip" size={14} /></span>
        <button class="pick" onclick={() => (downloads.playable(t) ? player.jumpTo(qi(k)) : ui.say(`“${t.title}” isn't downloaded. Connect to the internet to play it.`))} aria-label="Play {t.title}">
          <span class="thumb"><Cover art={t.art} radius={4} size={120} label={t.title} /></span>
          <span class="txt">
            <span class="t ell">{t.title}</span>
            <span class="a ell">{artistLine(t)}</span>
          </span>
        </button>
        <DownloadMark {id} size={13} />
        <span class="dur mono">{fmt(t.duration)}</span>
        <button class="icon-btn rm" aria-label="Remove {t.title} from queue" onclick={() => player.removeAt(qi(k))}><Icon name="x" size={15} /></button>
      </li>
    {:else}
      <li class="empty">Nothing queued. Right-click any song and choose Play next or Add to queue.</li>
    {/each}
  </ol>

  <Autoplay />
</section>

<style>
  .queue header {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    padding-bottom: 8px;
    border-bottom: 1px solid var(--line);
  }
  h3 {
    font-size: 13px;
    font-weight: 700;
  }
  .from {
    font-size: 12.5px;
    color: var(--mute);
  }
  .from button,
  .from span {
    color: var(--text);
    font-weight: 550;
  }
  .clear {
    font-size: 12.5px;
    font-weight: 600;
    color: var(--mute);
    padding: 4px 8px;
    border-radius: 6px;
  }
  .clear:hover {
    color: var(--text);
    background: var(--hover);
  }
  ol {
    list-style: none;
    margin: 6px -8px 0;
    padding: 0;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 6px 4px 0;
    border-radius: 8px;
    position: relative;
  }
  .row:hover {
    background: var(--hover);
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
  .row.dragging {
    opacity: 0.4;
  }
  /* Offline and not downloaded: skipped when the queue gets there (D23). */
  .row.unavailable .pick {
    opacity: 0.45;
  }
  .grip {
    width: 16px;
    display: grid;
    place-items: center;
    color: var(--dim);
    opacity: 0;
    cursor: grab;
  }
  .row:hover .grip {
    opacity: 1;
  }
  .pick {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 10px;
    text-align: left;
  }
  .thumb {
    width: 38px;
    flex: none;
  }
  .txt {
    display: grid;
    min-width: 0;
  }
  .t {
    font-size: 13.5px;
    font-weight: 550;
  }
  .a {
    font-size: 12.5px;
    color: var(--mute);
  }
  .dur {
    font-size: 11.5px;
    color: var(--mute);
  }
  .rm {
    width: 26px;
    height: 26px;
    opacity: 0;
  }
  .row:hover .rm,
  .rm:focus-visible {
    opacity: 1;
  }
  .row:hover .dur {
    display: none;
  }
  .empty {
    padding: 14px 8px;
    color: var(--mute);
    font-size: 13px;
    text-wrap: pretty;
  }
  .from button:hover {
    color: var(--text);
    text-decoration: underline;
    text-underline-offset: 3px;
  }
</style>
