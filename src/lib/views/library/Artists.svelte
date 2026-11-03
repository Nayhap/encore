<script lang="ts">
  // Library → Artists.
  import type { ShelfItem } from "$lib/backend";
  import Tile from "$lib/components/Tile.svelte";

  let { items }: { items: ShelfItem[] } = $props();
</script>

<div class="grid">
  {#each items as a (a.type + ("id" in a ? a.id : ""))}
    <Tile item={a} size={0} />
  {/each}
</div>

<style>
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(168px, 1fr));
    gap: 28px 20px;
  }
  .grid :global(.tile) {
    width: auto !important;
  }
</style>
