<script lang="ts">
  import type { PageLink, Shelf as ShelfT, Track } from "$lib/backend";
  import { nav } from "$lib/state.svelte";
  import Shelf from "./Shelf.svelte";
  import Tile from "./Tile.svelte";
  import TrackList from "./TrackList.svelte";

  // Renders backend shelves with the existing components: carousels as
  // Shelf + Tile, song lists as the ranked list, link grids as mood buttons.
  let { shelves }: { shelves: ShelfT[] } = $props();

  const open = (l: PageLink) => nav.go({ name: "page", browseId: l.browseId, params: l.params, title: l.title });
  const tracksOf = (s: ShelfT) => s.items.filter((i) => i.type === "track") as Track[];
  const isLinks = (s: ShelfT) => s.items.length > 0 && s.items.every((i) => i.type === "link");
  const isTrackList = (s: ShelfT) => s.layout === "list" && s.items.every((i) => i.type === "track");
</script>

{#each shelves as s, si (si + ":" + s.title)}
  {#if isLinks(s)}
    <section class="links">
      {#if s.title}<h2>{s.title}</h2>{/if}
      <div class="moods">
        {#each s.items as it}
          {#if it.type === "link"}
            <button class="mood" style:--mh={it.hue ?? 210} onclick={() => open(it)}>{it.title}</button>
          {/if}
        {/each}
      </div>
    </section>
  {:else if isTrackList(s)}
    <TrackList title={s.title || "Songs"} sub={s.subtitle} tracks={tracksOf(s)} ranked={false} />
  {:else}
    <Shelf title={s.title} sub={s.subtitle ?? undefined} onall={s.more ? () => open(s.more!) : undefined}>
      {#each s.items as it, i (i)}
        {#if it.type !== "link"}
          <Tile item={it} size={it.type === "artist" ? 168 : 176} />
        {/if}
      {/each}
    </Shelf>
  {/if}
{/each}

<style>
  .links {
    margin-top: 38px;
  }
  h2 {
    font-family: var(--font-display);
    font-size: 23px;
    font-weight: 650;
    letter-spacing: -0.02em;
    margin-bottom: 14px;
  }
  .moods {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    gap: 10px;
  }
  .mood {
    position: relative;
    height: 48px;
    padding: 0 16px 0 20px;
    border-radius: 10px;
    text-align: left;
    font-weight: 600;
    background: var(--hover);
    overflow: hidden;
  }
  .mood::before {
    content: "";
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    width: 5px;
    background: hsl(var(--mh) 70% 62%);
  }
  .mood:hover {
    background: hsl(var(--mh) 70% 55% / 0.16);
  }
</style>
