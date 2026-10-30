<script lang="ts">
  import { artistLine, fmt } from "$lib/data";
  import { nav, player, trackMenu, ui } from "$lib/state.svelte";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";
  import Lyrics from "./Lyrics.svelte";
  import VersionSwitch from "./VersionSwitch.svelte";

  const tr = $derived(player.current);
  const lyr = $derived(player.currentLyrics);
  const hasLyrics = $derived(!!lyr && (lyr.lines.length > 0 || !!lyr.plain));
  let wantLyrics = $state(true);
  const showLyrics = $derived(wantLyrics && hasLyrics);
  const artistId = $derived(tr?.artists.find((a) => a.id)?.id);

  let dragFrom = $state<number | null>(null);
  let dragOver = $state<number | null>(null);

  const qi = (k: number) => (player.index ?? -1) + 1 + k;

  function drop(k: number) {
    if (dragFrom !== null) player.move(qi(dragFrom), qi(k));
    dragFrom = dragOver = null;
  }
</script>

<aside class="stage" aria-label="Now playing">
  {#if tr}
    <div class="ambient" aria-hidden="true"></div>
    <div class="scroll">
      <div class="card" class:lyrics={showLyrics}>
        <div class="card-art"><Cover art={tr.art} radius={14} label={tr.title} /></div>
        {#if showLyrics}
          <div class="card-lyrics">
            <Lyrics lyrics={lyr} duration={player.duration ?? tr.duration ?? 0} position={player.position} onseek={(t) => player.seek(t)} />
          </div>
        {/if}
        <div class="card-tabs" role="group" aria-label="Show in panel">
          <button aria-pressed={showLyrics} disabled={!hasLyrics} title={hasLyrics ? "" : player.lyricsFor === tr.id ? "No lyrics for this song" : "Looking for lyrics…"} onclick={() => (wantLyrics = true)}>Lyrics</button>
          <button aria-pressed={!showLyrics} onclick={() => (wantLyrics = false)}>Artwork</button>
        </div>
        <button class="card-expand" aria-label="Full-screen lyrics" title="Full-screen lyrics (F)" onclick={() => (ui.theater = true)}><Icon name="expand" size={16} /></button>
      </div>

      <div class="head">
        <div class="titles">
          <h2>{tr.title}</h2>
          <p>
            <button onclick={() => artistId && nav.go({ name: "artist", id: artistId })}>{artistLine(tr)}</button>
            {#if tr.album}
              <span aria-hidden="true">·</span>
              <button onclick={() => tr.album?.id && nav.go({ name: "collection", id: tr.album.id })}>{tr.album.name}</button>
            {/if}
          </p>
        </div>
        <div class="tools">
          {#if player.currentEntry}<VersionSwitch entry={player.currentEntry} />{/if}
          <button
            class="icon-btn"
            class:on={player.liked.has(tr.id)}
            aria-pressed={player.liked.has(tr.id)}
            aria-label={player.liked.has(tr.id) ? "Remove from liked music" : "Add to liked music"}
            onclick={() => player.toggleLike(tr.id)}
          ><Icon name={player.liked.has(tr.id) ? "heartFill" : "heart"} size={20} /></button>
        </div>
      </div>

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
              <button class="pick" onclick={() => player.jumpTo(qi(k))} aria-label="Play {t.title}">
                <span class="thumb"><Cover art={t.art} radius={4} size={120} label={t.title} /></span>
                <span class="txt">
                  <span class="t ell">{t.title}</span>
                  <span class="a ell">{artistLine(t)}</span>
                </span>
              </button>
              <span class="dur mono">{fmt(t.duration)}</span>
              <button class="icon-btn rm" aria-label="Remove {t.title} from queue" onclick={() => player.removeAt(qi(k))}><Icon name="x" size={15} /></button>
            </li>
          {:else}
            <li class="empty">Nothing queued. Right-click any song and choose Play next or Add to queue.</li>
          {/each}
        </ol>

        <label class="autoplay">
          <span>
            <b>Autoplay</b>
            <small>Keep playing similar songs when the queue ends</small>
          </span>
          <input type="checkbox" role="switch" checked={player.autoplay} onchange={(e) => player.setAutoplay(e.currentTarget.checked)} />
        </label>
      </section>
    </div>
  {:else}
    <div class="scroll">
      <div class="idle-card">
        <div class="idle-art" aria-hidden="true"><Icon name="lyrics" size={34} /></div>
        <h2>Nothing playing yet</h2>
        <p>Play a song and it shows up here, with its lyrics and what's coming next.</p>
        <div class="idle-actions">
          <button class="pill primary" onclick={() => (ui.palette = true)}><Icon name="search" size={15} /> Search</button>
          <button class="pill" onclick={() => nav.go({ name: "explore" })}><Icon name="compass" size={15} /> Explore</button>
        </div>
      </div>
      <label class="autoplay">
        <span>
          <b>Autoplay</b>
          <small>Keep playing similar songs when the queue ends</small>
        </span>
        <input type="checkbox" role="switch" checked={player.autoplay} onchange={(e) => player.setAutoplay(e.currentTarget.checked)} />
      </label>
    </div>
  {/if}
</aside>

<style>
  .stage {
    grid-area: stage;
    position: relative;
    overflow: hidden;
    border-left: 1px solid var(--line);
    background: var(--panel);
  }
  .ambient {
    position: absolute;
    inset: 0;
    background:
      radial-gradient(120% 60% at 0% 0%, var(--tint-a), transparent 70%),
      radial-gradient(100% 50% at 100% 30%, var(--tint-b), transparent 70%);
    pointer-events: none;
  }
  .scroll {
    position: relative;
    height: 100%;
    overflow-y: auto;
    padding: calc(var(--top-h) + 2px) 18px 20px;
  }

  .card {
    position: relative;
    aspect-ratio: 1;
    border-radius: 14px;
    overflow: hidden;
    box-shadow: var(--shadow);
    isolation: isolate;
  }
  .card-art {
    position: absolute;
    inset: 0;
    transition: filter 0.5s var(--ease), transform 0.5s var(--ease);
  }
  .card.lyrics .card-art {
    filter: blur(28px) saturate(1.3) brightness(0.55);
    transform: scale(1.3);
  }
  :global([data-theme="light"]) .card.lyrics .card-art {
    filter: blur(28px) saturate(1.2) brightness(1.25) contrast(0.7);
  }
  .card-lyrics {
    position: absolute;
    inset: 48px 0 0;
    padding: 0 22px;
  }
  .card-tabs {
    position: absolute;
    left: 10px;
    top: 10px;
    display: flex;
    padding: 3px;
    gap: 2px;
    border-radius: 99px;
    background: rgb(0 0 0 / 0.35);
    backdrop-filter: blur(12px);
    -webkit-backdrop-filter: blur(12px);
    z-index: 2;
  }
  .card-tabs button {
    height: 24px;
    padding: 0 10px;
    border-radius: 99px;
    font-size: 12px;
    font-weight: 600;
    color: rgb(255 255 255 / 0.75);
  }
  .card-tabs button[aria-pressed="true"] {
    background: rgb(255 255 255 / 0.92);
    color: #111;
  }
  .card-tabs button:disabled {
    opacity: 0.4;
  }
  .card-expand {
    position: absolute;
    right: 10px;
    top: 10px;
    width: 30px;
    height: 30px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    color: #fff;
    background: rgb(0 0 0 / 0.35);
    backdrop-filter: blur(12px);
    -webkit-backdrop-filter: blur(12px);
    z-index: 2;
  }
  .card.lyrics :global(.line) {
    --lit: #fff !important;
  }
  .card.lyrics {
    color: #fff;
    --text: #fff;
  }
  :global([data-theme="light"]) .card.lyrics {
    color: #111;
    --text: #111;
  }
  :global([data-theme="light"]) .card.lyrics :global(.line) {
    --lit: #111 !important;
  }

  .head {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    margin: 16px 0 18px;
  }
  .titles {
    flex: 1;
    min-width: 0;
  }
  .tools {
    display: flex;
    align-items: center;
    gap: 4px;
    flex: none;
  }
  h2 {
    font-family: var(--font-display);
    font-size: 22px;
    font-weight: 650;
    line-height: 1.15;
    letter-spacing: -0.02em;
    text-wrap: balance;
  }
  .titles p {
    margin-top: 4px;
    color: var(--mute);
    font-size: 13.5px;
  }
  .titles p button:hover,
  .from button:hover {
    color: var(--text);
    text-decoration: underline;
    text-underline-offset: 3px;
  }

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
  .idle-card {
    display: grid;
    justify-items: center;
    gap: 10px;
    padding: 36px 20px 28px;
    border-radius: 14px;
    text-align: center;
    background: color-mix(in srgb, var(--text) 4%, transparent);
    border: 1px dashed color-mix(in srgb, var(--text) 14%, transparent);
  }
  .idle-art {
    display: grid;
    place-items: center;
    width: 72px;
    height: 72px;
    margin-bottom: 4px;
    border-radius: 18px;
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .idle-card h2 {
    margin: 0;
    font-size: 17px;
  }
  .idle-card p {
    margin: 0;
    max-width: 260px;
    color: var(--mute);
    font-size: 13px;
    line-height: 1.45;
    text-wrap: pretty;
  }
  .idle-actions {
    display: flex;
    gap: 8px;
    margin-top: 8px;
  }
  .idle-actions .pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }

  .empty {
    padding: 14px 8px;
    color: var(--mute);
    font-size: 13px;
    text-wrap: pretty;
  }

  .autoplay {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin-top: 12px;
    padding: 12px 0 0;
    border-top: 1px solid var(--line);
  }
  .autoplay span {
    display: grid;
  }
  .autoplay b {
    font-size: 13px;
  }
  .autoplay small {
    font-size: 12px;
    color: var(--mute);
  }
</style>
