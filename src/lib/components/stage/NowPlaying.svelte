<script lang="ts">
  // The now playing card (lyrics or artwork) and the song's title line.
  import type { Track } from "$lib/backend";
  import { artistLine } from "$lib/data";
  import { needsOnline, nav, player, ui } from "$lib/state.svelte";
  import Cover from "../Cover.svelte";
  import Icon from "../Icon.svelte";
  import Lyrics from "../Lyrics.svelte";
  import VersionSwitch from "../VersionSwitch.svelte";

  let { tr }: { tr: Track } = $props();

  const lyr = $derived(player.currentLyrics);
  const hasLyrics = $derived(!!lyr && (lyr.lines.length > 0 || !!lyr.plain));
  let wantLyrics = $state(true);
  const showLyrics = $derived(wantLyrics && hasLyrics);
  const artistId = $derived(tr.artists.find((a) => a.id)?.id);
</script>

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
      title={needsOnline("change your likes")}
      disabled={!ui.online}
      onclick={() => player.toggleLike(tr.id)}
    ><Icon name={player.liked.has(tr.id) ? "heartFill" : "heart"} size={20} /></button>
  </div>
</div>

<style>
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
  /* WebKitGTK may redo a blur this large on the CPU for every lyric repaint:
     a tint of the cover's colour instead (D17). */
  :global(:root[data-os="linux"]) .card.lyrics {
    background: color-mix(in srgb, var(--accent) 30%, #111);
  }
  :global(:root[data-os="linux"][data-theme="light"]) .card.lyrics {
    background: color-mix(in srgb, var(--accent) 22%, #fff);
  }
  :global(:root[data-os="linux"]) .card.lyrics .card-art {
    opacity: 0;
    filter: none;
  }
  :global(:root[data-os="linux"]) .card-tabs,
  :global(:root[data-os="linux"]) .card-expand {
    background: rgb(0 0 0 / 0.55);
    backdrop-filter: none;
    -webkit-backdrop-filter: none;
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
  .titles p button:hover {
    color: var(--text);
    text-decoration: underline;
    text-underline-offset: 3px;
  }
</style>
