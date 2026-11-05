<script lang="ts">
  import { artistLine, fmt } from "$lib/data";
  import { nav, player, ui } from "$lib/state.svelte";
  import Cover from "./Cover.svelte";
  import CoverWash from "./CoverWash.svelte";
  import Icon from "./Icon.svelte";
  import Lyrics from "./Lyrics.svelte";
  import SongMap from "./SongMap.svelte";

  const tr = $derived(player.current!);
  const dur = $derived(player.duration ?? tr.duration ?? 0);
  const artistId = $derived(tr.artists.find((a) => a.id)?.id);
</script>

<div class="theater" role="dialog" aria-modal="true" aria-label="Full-screen lyrics">
  <div class="bg" aria-hidden="true">
    <div class="blob a"></div>
    <div class="blob b"></div>
    <CoverWash strength="theater" />
  </div>
  <div class="bar" data-tauri-drag-region>
    <button class="close" onclick={() => (ui.theater = false)}>
      <Icon name="down" size={18} />
      Close <kbd>esc</kbd>
    </button>
  </div>

  <div class="grid">
    <section class="left">
      <div class="art"><Cover art={tr.art} radius={18} size={1080} label={tr.title} /></div>
      <div class="meta">
        <div>
          <h1>{tr.title}</h1>
          <p>
            <button onclick={() => { if (!artistId) return; ui.theater = false; nav.go({ name: "artist", id: artistId }); }}>{artistLine(tr)}</button>
            {#if tr.album}· {tr.album.name}{/if}
          </p>
        </div>
        <button class="icon-btn like" class:on={player.liked.has(tr.id)} aria-pressed={player.liked.has(tr.id)} aria-label="Like" onclick={() => player.toggleLike(tr.id)}>
          <Icon name={player.liked.has(tr.id) ? "heartFill" : "heart"} size={22} />
        </button>
      </div>
      <div class="map">
        <SongMap lines={player.timedLines} duration={dur} position={player.position} onseek={(t) => player.seek(t)} size="large" />
        <div class="times mono"><span>{fmt(player.position)}</span><span>-{fmt(dur - player.position)}</span></div>
      </div>
      <div class="controls">
        <button class="icon-btn" class:on={player.shuffle} aria-pressed={player.shuffle} aria-label="Shuffle" onclick={() => player.toggleShuffle()}><Icon name="shuffle" size={20} /></button>
        <button class="icon-btn big" aria-label="Previous" onclick={() => player.prev()}><Icon name="prev" size={26} /></button>
        <button class="play" aria-label={player.playing ? "Pause" : "Play"} onclick={() => player.toggle()}><Icon name={player.playing ? "pause" : "play"} size={28} /></button>
        <button class="icon-btn big" aria-label="Next" onclick={() => player.next()}><Icon name="next" size={26} /></button>
        <button class="icon-btn" class:on={player.repeat !== "off"} aria-label="Repeat: {player.repeat}" onclick={() => player.cycleRepeat()}>
          <Icon name="repeat" size={20} />{#if player.repeat === "one"}<b class="one">1</b>{/if}
        </button>
      </div>
    </section>
    <section class="right">
      <Lyrics lyrics={player.currentLyrics} duration={dur} position={player.position} onseek={(t) => player.seek(t)} size="theater" loading={player.lyricsFor !== tr.id} />
    </section>
  </div>
</div>

<style>
  .theater {
    position: fixed;
    inset: 0;
    z-index: 40;
    background: var(--ink);
    animation: enter 0.35s var(--ease);
    overflow: hidden;
  }
  .bg {
    position: absolute;
    inset: 0;
    overflow: hidden;
  }
  /* Two soft lights. Each is the 80vmax disc, blurred by 90px, that it once
     was, drawn as the radial gradient a Gaussian blur makes of a disc (the
     normal CDF across its edge: 50 % at the rim, out to 3σ = 270px), so the
     compositor has no full-screen blur to redo while the theater is open. */
  .blob {
    --r: 40vmax;
    --spread: 270px;
    position: absolute;
    width: calc(2 * (var(--r) + var(--spread)));
    height: calc(2 * (var(--r) + var(--spread)));
    margin: calc(-1 * var(--spread));
    border-radius: 50%;
    opacity: 0.9;
  }
  .blob.a {
    left: -25vmax;
    top: -35vmax;
    background: radial-gradient(
      circle closest-side,
      var(--tint-a) calc(var(--r) - 180px),
      color-mix(in srgb, var(--tint-a) 84.1%, transparent) calc(var(--r) - 90px),
      color-mix(in srgb, var(--tint-a) 69.1%, transparent) calc(var(--r) - 45px),
      color-mix(in srgb, var(--tint-a) 50%, transparent) var(--r),
      color-mix(in srgb, var(--tint-a) 30.9%, transparent) calc(var(--r) + 45px),
      color-mix(in srgb, var(--tint-a) 15.9%, transparent) calc(var(--r) + 90px),
      color-mix(in srgb, var(--tint-a) 6.7%, transparent) calc(var(--r) + 135px),
      color-mix(in srgb, var(--tint-a) 2.3%, transparent) calc(var(--r) + 180px),
      transparent calc(var(--r) + 270px)
    );
    animation: drift 28s ease-in-out infinite alternate;
  }
  .blob.b {
    right: -30vmax;
    bottom: -40vmax;
    background: radial-gradient(
      circle closest-side,
      var(--tint-b) calc(var(--r) - 180px),
      color-mix(in srgb, var(--tint-b) 84.1%, transparent) calc(var(--r) - 90px),
      color-mix(in srgb, var(--tint-b) 69.1%, transparent) calc(var(--r) - 45px),
      color-mix(in srgb, var(--tint-b) 50%, transparent) var(--r),
      color-mix(in srgb, var(--tint-b) 30.9%, transparent) calc(var(--r) + 45px),
      color-mix(in srgb, var(--tint-b) 15.9%, transparent) calc(var(--r) + 90px),
      color-mix(in srgb, var(--tint-b) 6.7%, transparent) calc(var(--r) + 135px),
      color-mix(in srgb, var(--tint-b) 2.3%, transparent) calc(var(--r) + 180px),
      transparent calc(var(--r) + 270px)
    );
    animation: drift 34s ease-in-out infinite alternate-reverse;
  }
  @keyframes drift {
    to {
      transform: translate(8vmax, 6vmax) scale(1.1);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .blob {
      animation: none;
    }
  }
  /* Linux keeps its own still, plainer glow (D17). */
  :global(:root[data-os="linux"]) .blob {
    --spread: 0px;
    animation: none;
  }
  :global(:root[data-os="linux"]) .blob.a {
    background: radial-gradient(closest-side, var(--tint-a), transparent);
  }
  :global(:root[data-os="linux"]) .blob.b {
    background: radial-gradient(closest-side, var(--tint-b), transparent);
  }
  .bar {
    position: relative;
    height: var(--top-h);
    display: flex;
    justify-content: flex-end;
    align-items: center;
    padding: 0 calc(18px + var(--wc-w)) 0 18px;
    z-index: 2;
  }
  .close {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 32px;
    padding: 0 10px 0 12px;
    border-radius: 99px;
    font-size: 13px;
    font-weight: 600;
    background: var(--hover);
  }
  .close:hover {
    background: color-mix(in srgb, var(--text) 14%, transparent);
  }

  .grid {
    position: relative;
    height: calc(100% - var(--top-h));
    display: grid;
    grid-template-columns: minmax(300px, 0.9fr) 1.2fr;
    gap: clamp(32px, 6vw, 96px);
    padding: 0 clamp(32px, 6vw, 96px) 40px;
  }
  .left {
    align-self: center;
    display: grid;
    gap: 22px;
    max-width: 480px;
    width: 100%;
    justify-self: end;
  }
  .art {
    width: min(100%, 52vh);
    box-shadow: 0 30px 80px -20px rgb(0 0 0 / 0.55);
    border-radius: 18px;
  }
  .meta {
    display: flex;
    align-items: flex-start;
    gap: 12px;
  }
  .meta > div {
    flex: 1;
    min-width: 0;
  }
  h1 {
    font-family: var(--font-display);
    font-size: clamp(26px, 2.6vw, 36px);
    font-weight: 700;
    line-height: 1.08;
    letter-spacing: -0.025em;
    text-wrap: balance;
  }
  .meta p {
    margin-top: 6px;
    color: var(--mute);
    font-size: 15px;
  }
  .meta p button:hover {
    color: var(--text);
    text-decoration: underline;
  }
  .like {
    width: 40px;
    height: 40px;
  }
  .times {
    display: flex;
    justify-content: space-between;
    font-size: 12px;
    color: var(--mute);
    margin-top: 4px;
  }
  .controls {
    display: flex;
    align-items: center;
    justify-content: space-between;
    max-width: 360px;
  }
  .controls .icon-btn {
    position: relative;
    width: 44px;
    height: 44px;
  }
  .play {
    display: grid;
    place-items: center;
    width: 64px;
    height: 64px;
    border-radius: 50%;
    background: var(--text);
    color: var(--ink);
    transition: transform 0.15s var(--ease);
  }
  .play:hover {
    transform: scale(1.05);
  }
  .big {
    color: var(--text) !important;
  }
  .one {
    position: absolute;
    top: 4px;
    right: 3px;
    min-width: 14px;
    height: 14px;
    border-radius: 7px;
    font: 700 9px/14px var(--font-mono);
    background: var(--accent);
    color: var(--accent-ink);
  }
  .right {
    min-height: 0;
  }

  @keyframes enter {
    from {
      opacity: 0;
      transform: translateY(24px);
    }
  }

  @media (max-width: 900px) {
    .grid {
      grid-template-columns: 1fr;
      grid-template-rows: auto 1fr;
    }
    .left {
      justify-self: stretch;
      max-width: none;
      grid-template-columns: 120px 1fr;
      align-items: center;
    }
    .art {
      width: 120px;
    }
    .map,
    .controls {
      grid-column: 1 / -1;
    }
  }
</style>
