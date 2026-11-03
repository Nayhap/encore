<script lang="ts">
  // Library → Playlists and Albums, as tiles or as a list.
  import type { CollectionCard } from "$lib/backend";
  import { kindLabel } from "$lib/data";
  import { collectionMenu, nav, newPlaylist, player, ui } from "$lib/state.svelte";
  import Cover from "$lib/components/Cover.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import Tile from "$lib/components/Tile.svelte";

  let { cols, layout, playlists }: { cols: (CollectionCard & { type: "collection" })[]; layout: "grid" | "list"; playlists: boolean } = $props();
</script>

{#if layout === "grid"}
  <div class="grid">
    {#if playlists}
      <button class="new" onclick={() => newPlaylist()}>
        <span class="new-art"><Icon name="plus" size={30} stroke={1.5} /></span>
        <span class="t">New playlist</span>
      </button>
    {/if}
    {#each cols as c (c.id)}
      <Tile item={c} size={0} />
    {/each}
  </div>
{:else}
  <ul class="list">
    {#if playlists}
      <li>
        <button onclick={() => newPlaylist()}>
          <span class="thumb new-thumb"><Icon name="plus" size={22} stroke={1.5} /></span>
          <span class="txt"><span class="t ell">New playlist</span></span>
        </button>
      </li>
    {/if}
    {#each cols as c (c.id)}
      <li>
        <button onclick={() => nav.go({ name: "collection", id: c.id })} oncontextmenu={(e) => ui.openMenu(e, collectionMenu(c.id))}>
          <span class="thumb"><Cover art={c.art} radius={6} size={120} label={c.title} /></span>
          <span class="txt">
            <span class="t ell">{c.title}</span>
            <span class="s ell">{c.subtitle || kindLabel[c.kind]}</span>
          </span>
          {#if player.source?.collectionId === c.id}<span class="playing">Playing</span>{/if}
        </button>
      </li>
    {/each}
  </ul>
{/if}

<style>
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(168px, 1fr));
    gap: 28px 20px;
  }
  .grid :global(.tile) {
    width: auto !important;
  }
  .new {
    display: grid;
    align-content: start;
    gap: 10px;
    text-align: left;
  }
  .new-art {
    aspect-ratio: 1;
    display: grid;
    place-items: center;
    border-radius: 10px;
    border: 1.5px dashed color-mix(in srgb, var(--text) 25%, transparent);
    color: var(--mute);
    transition: border-color 0.15s, color 0.15s;
  }
  .new:hover .new-art {
    border-color: var(--accent);
    color: var(--accent);
  }
  .t {
    font-weight: 600;
  }
  .list {
    list-style: none;
    margin: 0 -8px;
    padding: 0;
  }
  .list button {
    display: flex;
    align-items: center;
    gap: 14px;
    width: 100%;
    padding: 7px 8px;
    border-radius: 9px;
    text-align: left;
  }
  .list button:hover {
    background: var(--hover);
  }
  .thumb {
    width: 52px;
    flex: none;
  }
  .new-thumb {
    aspect-ratio: 1;
    display: grid;
    place-items: center;
    border-radius: 6px;
    border: 1.5px dashed color-mix(in srgb, var(--text) 25%, transparent);
    color: var(--mute);
  }
  .list button:hover .new-thumb {
    border-color: var(--accent);
    color: var(--accent);
  }
  .txt {
    flex: 1;
    display: grid;
    min-width: 0;
  }
  .s {
    font-size: 13px;
    color: var(--mute);
  }
  .playing {
    font-size: 12px;
    font-weight: 600;
    color: var(--accent);
  }
</style>
