<script lang="ts">
  // A folder's cover (D34): the picture chosen for it, else the covers of
  // its first four lists, else a folder glyph.
  import type { Folder } from "$lib/backend";
  import Cover from "../Cover.svelte";
  import Icon from "../Icon.svelte";

  let { folder, radius = 8, size = 226 }: { folder: Folder; radius?: number; size?: number } = $props();
  const arts = $derived(folder.items.map((i) => i.card.art).filter((a) => !!a).slice(0, 4));
</script>

<span class="fc" style:border-radius="{radius}px">
  {#if folder.cover}
    <Cover art={{ key: folder.cover, h1: null, h2: null }} {radius} {size} label={folder.name} />
  {:else if arts.length >= 4}
    <span class="grid">
      {#each arts as a}<Cover art={a} radius={0} size={size > 200 ? 226 : 120} label="" />{/each}
    </span>
  {:else if arts.length}
    <Cover art={arts[0]} {radius} {size} label={folder.name} />
    <span class="badge" aria-hidden="true"><Icon name="folder" size={Math.min(22, Math.max(11, Math.round(size / 14)))} stroke={2} /></span>
  {:else}
    <span class="none" aria-hidden="true"><Icon name="folder" size={Math.max(16, Math.round(size / 6))} stroke={1.5} /></span>
  {/if}
</span>

<style>
  .fc {
    position: relative;
    display: block;
    aspect-ratio: 1;
    overflow: hidden;
    background: color-mix(in srgb, var(--text) 6%, transparent);
  }
  .grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    width: 100%;
    height: 100%;
  }
  .none {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    color: var(--mute);
  }
  .badge {
    position: absolute;
    right: 6%;
    bottom: 6%;
    display: grid;
    place-items: center;
    padding: 4%;
    border-radius: 30%;
    color: #fff;
    background: rgb(0 0 0 / 0.45);
    backdrop-filter: blur(6px);
    -webkit-backdrop-filter: blur(6px);
  }
</style>
