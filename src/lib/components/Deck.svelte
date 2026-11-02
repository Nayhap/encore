<script lang="ts">
  import { mod } from "$lib/platform";
  import { artistLine, fmt } from "$lib/data";
  import { nav, player, trackMenu, ui } from "$lib/state.svelte";
  import Cover from "./Cover.svelte";
  import DownloadMark from "./DownloadMark.svelte";
  import Icon from "./Icon.svelte";
  import SongMap from "./SongMap.svelte";

  const tr = $derived(player.current);
  const dur = $derived(player.duration ?? tr?.duration ?? 0);
  const artistId = $derived(tr?.artists.find((a) => a.id)?.id);
  const vol = $derived(player.muted ? 0 : player.volume);
  const repeatLabel = $derived(
    player.repeat === "off" ? "Repeat is off" : player.repeat === "all" ? "Repeating the queue" : "Repeating this song",
  );
</script>

<footer class="deck">
  {#if tr}
    <div class="now">
      <button class="art" onclick={() => (ui.theater = true)} aria-label="Open full-screen lyrics">
        <Cover art={tr.art} radius={6} size={120} label={tr.title} />
        <span class="art-hint"><Icon name="expand" size={18} /></span>
      </button>
      <div class="meta">
        <button class="title ell" onclick={() => tr.album?.id && nav.go({ name: "collection", id: tr.album.id })} title={tr.title}>{tr.title}</button>
        {#if player.noOutput || (player.phase === "error" && player.error)}
          <!-- Say why nothing is heard, for as long as it's true (the
               notice that also says so goes away after a few seconds). -->
          <span class="status ell" role="status" title={player.noOutput ? "No audio output" : player.error}>
            {player.noOutput ? "No audio output: connect speakers or headphones" : player.error}
          </span>
        {:else}
          <button class="artist ell" onclick={() => artistId && nav.go({ name: "artist", id: artistId })}>{artistLine(tr)}</button>
        {/if}
      </div>
      <DownloadMark id={tr.id} size={15} />
      <button
        class="icon-btn"
        class:on={player.liked.has(tr.id)}
        aria-pressed={player.liked.has(tr.id)}
        aria-label={player.liked.has(tr.id) ? "Remove from liked music" : "Add to liked music"}
        disabled={!ui.online}
        onclick={() => player.toggleLike(tr.id)}
      ><Icon name={player.liked.has(tr.id) ? "heartFill" : "heart"} size={19} /></button>
      <button class="icon-btn more" aria-label="More actions" onclick={(e) => ui.openMenu(e, trackMenu(tr.id))}><Icon name="more" size={18} /></button>
    </div>

    <div class="transport">
      <div class="buttons">
        <button class="icon-btn" class:on={player.shuffle} aria-pressed={player.shuffle} aria-label="Shuffle" title="Shuffle (S)" onclick={() => player.toggleShuffle()}>
          <Icon name="shuffle" size={18} />
          {#if player.shuffle}<i class="dot"></i>{/if}
        </button>
        <button class="icon-btn" aria-label="Previous" title="Previous ({mod('←')})" onclick={() => player.prev()}><Icon name="prev" size={20} /></button>
        <button class="play" class:loading={player.phase === "loading" || player.phase === "buffering"} aria-label={player.playing ? "Pause" : "Play"} aria-busy={player.phase === "loading"} title="{player.phase === 'loading' ? 'Loading… ' : ''}{player.playing ? 'Pause' : 'Play'} (Space)" onclick={() => player.toggle()}>
          <Icon name={player.playing ? "pause" : "play"} size={20} />
        </button>
        <button class="icon-btn" aria-label="Next" title="Next ({mod('→')})" onclick={() => player.next()}><Icon name="next" size={20} /></button>
        <button class="icon-btn" class:on={player.repeat !== "off"} aria-label={repeatLabel} title="{repeatLabel} (R)" onclick={() => player.cycleRepeat()}>
          <Icon name="repeat" size={18} />
          {#if player.repeat === "one"}<b class="one">1</b>{:else if player.repeat === "all"}<i class="dot"></i>{/if}
        </button>
      </div>
      <div class="scrub">
        <span class="mono time">{fmt(player.position)}</span>
        <SongMap lines={player.timedLines} duration={dur} position={player.position} onseek={(t) => player.seek(t)} />
        <span class="mono time">{fmt(dur || null)}</span>
      </div>
    </div>

    <div class="tools">
      <button class="icon-btn" class:on={ui.theater} aria-label="Full-screen lyrics" title="Full-screen lyrics (F)" onclick={() => (ui.theater = !ui.theater)}><Icon name="lyrics" size={19} /></button>
      <button class="icon-btn" class:on={ui.stageOpen} aria-pressed={ui.stageOpen} aria-label="Now playing panel" title="Now playing panel ({mod('\\')})" onclick={() => (ui.stageOpen = !ui.stageOpen)}><Icon name="stage" size={19} /></button>
      <div class="volume">
        <button class="icon-btn" aria-label={player.muted ? "Unmute" : "Mute"} onclick={() => player.setMuted(!player.muted)}>
          <Icon name={vol === 0 ? "mute" : vol < 0.5 ? "volumeLow" : "volume"} size={19} />
        </button>
        <input
          type="range"
          min="0"
          max="1"
          step="0.01"
          value={vol}
          aria-label="Volume"
          style:--v="{vol * 100}%"
          oninput={(e) => player.setVolume(+e.currentTarget.value)}
          onwheel={(e) => { e.preventDefault(); player.setVolume(player.volume - e.deltaY / 1000); }}
        />
      </div>
    </div>
  {:else}
    <!-- Nothing loaded yet (a fresh install, a cleared queue): keep the bar's
         shape so the window doesn't look broken, and say how to start. -->
    <div class="now idle">
      <span class="art-empty" aria-hidden="true"><Icon name="lyrics" size={22} /></span>
      <div class="meta">
        <span class="title">Nothing playing</span>
        <button class="artist hint" onclick={() => (ui.palette = true)}>Search with {mod("K")} to start</button>
      </div>
    </div>
    <div class="transport idle" aria-hidden="true">
      <div class="buttons">
        <span class="icon-btn"><Icon name="shuffle" size={18} /></span>
        <span class="icon-btn"><Icon name="prev" size={20} /></span>
        <span class="play"><Icon name="play" size={20} /></span>
        <span class="icon-btn"><Icon name="next" size={20} /></span>
        <span class="icon-btn"><Icon name="repeat" size={18} /></span>
      </div>
      <div class="scrub">
        <span class="mono time">0:00</span>
        <span class="track-empty"></span>
        <span class="mono time">0:00</span>
      </div>
    </div>
    <div class="tools">
      <button class="icon-btn" class:on={ui.stageOpen} aria-pressed={ui.stageOpen} aria-label="Now playing panel" title="Now playing panel ({mod('\\')})" onclick={() => (ui.stageOpen = !ui.stageOpen)}><Icon name="stage" size={19} /></button>
      <div class="volume">
        <button class="icon-btn" aria-label={player.muted ? "Unmute" : "Mute"} onclick={() => player.setMuted(!player.muted)}>
          <Icon name={vol === 0 ? "mute" : vol < 0.5 ? "volumeLow" : "volume"} size={19} />
        </button>
        <input type="range" min="0" max="1" step="0.01" value={vol} aria-label="Volume" style:--v="{vol * 100}%" oninput={(e) => player.setVolume(+e.currentTarget.value)} />
      </div>
    </div>
  {/if}
</footer>

<style>
  .deck {
    grid-area: deck;
    height: var(--deck-h);
    display: grid;
    grid-template-columns: minmax(220px, 1fr) minmax(360px, 2.1fr) minmax(220px, 1fr);
    align-items: center;
    gap: 20px;
    padding: 0 18px 0 14px;
    border-top: 1px solid var(--line);
    background: var(--panel);
    position: relative;
    z-index: 3;
  }

  .art-empty {
    display: grid;
    place-items: center;
    width: 56px;
    height: 56px;
    flex: none;
    border-radius: 6px;
    color: var(--mute);
    background: color-mix(in srgb, var(--text) 7%, transparent);
    border: 1px dashed color-mix(in srgb, var(--text) 16%, transparent);
  }
  .idle .title {
    color: var(--mute);
  }
  .hint {
    text-align: left;
  }
  .transport.idle {
    opacity: 0.35;
    pointer-events: none;
  }
  .track-empty {
    flex: 1;
    height: 4px;
    border-radius: 4px;
    background: color-mix(in srgb, var(--text) 18%, transparent);
  }

  .now {
    display: flex;
    align-items: center;
    gap: 12px;
    min-width: 0;
  }
  .art {
    position: relative;
    width: 56px;
    flex: none;
    border-radius: 6px;
  }
  .art-hint {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    border-radius: 6px;
    background: rgb(0 0 0 / 0.45);
    color: #fff;
    opacity: 0;
    transition: opacity 0.15s;
  }
  .art:hover .art-hint,
  .art:focus-visible .art-hint {
    opacity: 1;
  }
  .meta {
    display: grid;
    min-width: 0;
    gap: 1px;
  }
  .meta button {
    text-align: left;
    max-width: 100%;
  }
  .meta button:hover {
    text-decoration: underline;
    text-underline-offset: 3px;
  }
  .title {
    font-weight: 600;
    font-size: 14px;
  }
  .artist {
    font-size: 13px;
    color: var(--mute);
  }
  .status {
    font-size: 12.5px;
    color: color-mix(in srgb, var(--accent) 70%, var(--text));
  }
  .now .more {
    opacity: 0;
  }
  .now:hover .more,
  .now .more:focus-visible {
    opacity: 1;
  }

  .transport {
    display: grid;
    gap: 2px;
    justify-items: center;
    min-width: 0;
  }
  .buttons {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .buttons .icon-btn {
    position: relative;
  }
  .play {
    position: relative;
    display: grid;
    place-items: center;
    width: 38px;
    height: 38px;
    border-radius: 50%;
    background: var(--text);
    color: var(--ink);
    transition: transform 0.15s var(--ease);
  }
  /* Getting a stream can take several seconds: show that something is
     happening instead of a play button that seems to do nothing. */
  .play.loading::after {
    content: "";
    position: absolute;
    inset: -4px;
    border-radius: 50%;
    border: 2px solid transparent;
    border-top-color: var(--text);
    animation: spin 0.9s linear infinite;
  }
  @media (prefers-reduced-motion: reduce) {
    .play.loading::after {
      animation: none !important;
      border-color: color-mix(in srgb, var(--text) 35%, transparent);
      border-top-color: var(--text);
      border-right-color: var(--text);
    }
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .play:hover {
    transform: scale(1.06);
  }
  .play:active {
    transform: scale(0.95);
  }
  .dot {
    position: absolute;
    bottom: 1px;
    left: 50%;
    width: 4px;
    height: 4px;
    margin-left: -2px;
    border-radius: 50%;
    background: currentColor;
  }
  .one {
    position: absolute;
    top: 2px;
    right: 1px;
    min-width: 13px;
    height: 13px;
    border-radius: 7px;
    font: 700 9px/13px var(--font-mono);
    background: var(--accent);
    color: var(--accent-ink);
  }
  .scrub {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
  }
  .time {
    font-size: 11.5px;
    color: var(--mute);
    width: 34px;
    flex: none;
  }
  .time:first-child {
    text-align: right;
  }

  .tools {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 4px;
  }
  .volume {
    display: flex;
    align-items: center;
    gap: 4px;
    margin-left: 6px;
  }
  input[type="range"] {
    -webkit-appearance: none;
    appearance: none;
    width: 96px;
    height: 16px;
    background: transparent;
  }
  input[type="range"]::-webkit-slider-runnable-track {
    height: 4px;
    border-radius: 4px;
    background: linear-gradient(90deg, var(--text) var(--v), color-mix(in srgb, var(--text) 18%, transparent) var(--v));
  }
  input[type="range"]::-webkit-slider-thumb {
    -webkit-appearance: none;
    width: 12px;
    height: 12px;
    margin-top: -4px;
    border-radius: 50%;
    background: var(--text);
    opacity: 0;
    transition: opacity 0.15s;
  }
  input[type="range"]:hover::-webkit-slider-thumb,
  input[type="range"]:focus-visible::-webkit-slider-thumb {
    opacity: 1;
  }
  input[type="range"]:hover::-webkit-slider-runnable-track {
    background: linear-gradient(90deg, var(--accent) var(--v), color-mix(in srgb, var(--text) 18%, transparent) var(--v));
  }

  @media (max-width: 1000px) {
    .deck {
      grid-template-columns: minmax(160px, 1fr) minmax(300px, 2fr) auto;
      gap: 12px;
    }
    .volume input {
      display: none;
    }
  }
</style>
