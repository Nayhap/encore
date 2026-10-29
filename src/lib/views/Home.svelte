<script lang="ts">
  import { call, type CollectionCard, type Page } from "$lib/backend";
  import { catalog } from "$lib/catalog.svelte";
  import { Resource } from "$lib/load.svelte";
  import { collectionMenu, nav, player, ui } from "$lib/state.svelte";
  import Cover from "$lib/components/Cover.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import Shelves from "$lib/components/Shelves.svelte";
  import Status from "$lib/components/Status.svelte";

  const hour = new Date().getHours();
  const title = hour < 5 ? "Up late" : hour < 12 ? "This morning" : hour < 18 ? "This afternoon" : "Tonight";

  let chip = $state<string | null>(null);
  const page = new Resource<Page>(async () => {
    const p = await call((a) => (chip ? a.browsePage("FEmusic_home", chip) : a.home()));
    catalog.addPage(p);
    return p;
  });
  let more = $state<Page["shelves"]>([]);
  let moreCursor = $state<string | null>(null);
  let loadingMore = $state(false);
  $effect(() => {
    if (page.data) {
      more = [];
      moreCursor = page.data.cursor;
    }
  });

  async function loadMore() {
    if (!moreCursor || loadingMore) return;
    loadingMore = true;
    try {
      const p = await call((a) => a.moreShelves(moreCursor!));
      catalog.addPage(p);
      more = [...more, ...p.shelves];
      moreCursor = p.cursor;
    } catch {
      moreCursor = null;
    } finally {
      loadingMore = false;
    }
  }

  function pick(c: string | null) {
    chip = chip === c ? null : c;
    void page.reload();
  }

  // The first shelf of playlists/albums becomes the quick-jump grid.
  const quickShelf = $derived(page.data?.shelves.find((s) => s.items.filter((i) => i.type === "collection").length >= 6) ?? null);
  const quick = $derived((quickShelf?.items.filter((i) => i.type === "collection") ?? []).slice(0, 8) as CollectionCard[]);
  const rest = $derived([...(page.data?.shelves.filter((s) => s !== quickShelf) ?? []), ...more]);
</script>

<div class="page">
  <h1 class="page-title">{title}</h1>

  {#if page.data?.chips.length}
    <div class="chips" role="group" aria-label="Filter by mood">
      <button class="chip" aria-pressed={chip === null} onclick={() => pick(null)}>All</button>
      {#each page.data.chips as c}
        <button class="chip" aria-pressed={chip === c.params} onclick={() => pick(c.params)}>{c.title}</button>
      {/each}
    </div>
  {/if}

  <Status loading={page.loading && !page.data} error={page.error} onretry={() => page.reload()} empty={page.data && !page.data.shelves.length ? "Nothing here yet. Play some music and YouTube Music will fill this in." : null} />

  {#if quick.length}
    <div class="quick-wrap"><div class="quick">
      {#each quick as c (c.id)}
        {@const cur = player.source?.collectionId === c.id}
        <div class="q" class:cur oncontextmenu={(e) => ui.openMenu(e, collectionMenu(c.id))} role="group" aria-label={c.title}>
          <button class="q-open" onclick={() => nav.go({ name: "collection", id: c.id })}>
            <span class="q-art"><Cover art={c.art} radius={0} size={120} label={c.title} /></span>
            <span class="q-t clamp2">{c.title}</span>
          </button>
          <button class="q-play" aria-label={cur && player.playing ? "Pause" : `Play ${c.title}`} onclick={() => (cur ? player.toggle() : player.playCollection(c.id))}>
            <Icon name={cur && player.playing ? "pause" : "play"} size={16} />
          </button>
        </div>
      {/each}
    </div></div>
  {/if}

  <Shelves shelves={rest} />

  {#if moreCursor}
    <div class="more"><button class="pill" disabled={loadingMore} onclick={loadMore}>{loadingMore ? "Loading…" : "Show more"}</button></div>
  {/if}
</div>

<style>
  .chips {
    display: flex;
    gap: 8px;
    margin: 18px 0 22px;
    flex-wrap: wrap;
  }

  /* Always whole rows: six tiles in three columns, or eight in four when wide. */
  .quick-wrap {
    container-type: inline-size;
  }
  .quick {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 10px;
  }
  .quick > :nth-child(n + 7) {
    display: none;
  }
  @container (min-width: 900px) {
    .quick {
      grid-template-columns: repeat(4, minmax(0, 1fr));
    }
    .quick > :nth-child(n + 7) {
      display: block;
    }
  }
  @container (max-width: 560px) {
    .quick {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
  }
  .q {
    position: relative;
    border-radius: 10px;
    background: var(--hover);
    overflow: hidden;
    transition: background-color 0.15s;
  }
  .q:hover {
    background: color-mix(in srgb, var(--text) 12%, transparent);
  }
  .q-open {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    height: 60px;
    padding-right: 52px;
    text-align: left;
  }
  .q-art {
    width: 60px;
    flex: none;
  }
  .q-t {
    font-weight: 600;
    font-size: 13.5px;
    line-height: 1.25;
  }
  .q.cur .q-t {
    color: var(--accent);
  }
  .q-play {
    position: absolute;
    right: 10px;
    top: 50%;
    width: 34px;
    height: 34px;
    margin-top: -17px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    background: var(--accent);
    color: var(--accent-ink);
    opacity: 0;
    transition: opacity 0.15s;
  }
  .q:hover .q-play,
  .q-play:focus-visible,
  .q.cur .q-play {
    opacity: 1;
  }

  .more {
    display: flex;
    justify-content: center;
    margin-top: 32px;
  }
</style>
