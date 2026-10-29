<script lang="ts">
  import type { Track } from "$lib/backend";
  import { artistLine } from "$lib/data";
  import { player, trackMenu, ui } from "$lib/state.svelte";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";

  // The ranked song list (Home "Top songs" / "Quick picks" style).
  let { title, sub, tracks, ranked = true }: { title: string; sub?: string | null; tracks: Track[]; ranked?: boolean } = $props();
  const source = $derived({ label: title || "Songs", collectionId: null, radioPlaylist: null });
</script>

<section class="chart">
  <header>
    <div>
      {#if sub}<p class="sub">{sub}</p>{/if}
      <h2>{title}</h2>
    </div>
    <button class="pill" onclick={() => player.playTracks(tracks.filter((t) => t.playable), 0, source)}><Icon name="play" size={15} />Play all</button>
  </header>
  <ol>
    {#each tracks as t, i (t.id + ":" + i)}
      {@const cur = player.current?.id === t.id}
      <li class:cur oncontextmenu={(e) => ui.openMenu(e, trackMenu(t.id))}>
        <button onclick={() => player.playTracks(tracks.filter((x) => x.playable), tracks.slice(0, i).filter((x) => x.playable).length, source)}>
          {#if ranked}<span class="rank mono">{i + 1}</span>{/if}
          <span class="c-art"><Cover art={t.art} radius={5} size={120} label={t.title} /></span>
          <span class="c-txt">
            <span class="ell c-t">{t.title}</span>
            <span class="ell c-a">{artistLine(t)}{t.plays ? ` · ${t.plays}` : ""}</span>
          </span>
        </button>
      </li>
    {/each}
  </ol>
</section>

<style>
  .chart {
    margin-top: 38px;
  }
  .chart header {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    margin-bottom: 12px;
  }
  .sub {
    font-size: 12.5px;
    color: var(--mute);
    margin-bottom: 2px;
  }
  h2 {
    font-family: var(--font-display);
    font-size: 23px;
    font-weight: 650;
    letter-spacing: -0.02em;
  }
  ol {
    list-style: none;
    margin: 0 -8px;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    grid-auto-flow: row;
    column-gap: 16px;
  }
  li button {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    padding: 6px 8px;
    border-radius: 8px;
    text-align: left;
  }
  li button:hover {
    background: var(--hover);
  }
  .rank {
    width: 22px;
    text-align: right;
    font-size: 15px;
    font-weight: 500;
    color: var(--mute);
  }
  .c-art {
    width: 44px;
    flex: none;
  }
  .c-txt {
    display: grid;
    min-width: 0;
  }
  .c-t {
    font-weight: 550;
  }
  li.cur .c-t,
  li.cur .rank {
    color: var(--accent);
  }
  .c-a {
    font-size: 12.5px;
    color: var(--mute);
  }
</style>
