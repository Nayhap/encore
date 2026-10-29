<script lang="ts">
  import { call, type CollectionCard, type LibraryPage, type LibrarySection, type ShelfItem, type Track } from "$lib/backend";
  import { catalog } from "$lib/catalog.svelte";
  import { kindLabel } from "$lib/data";
  import { Resource } from "$lib/load.svelte";
  import { account, collectionMenu, nav, player, ui } from "$lib/state.svelte";
  import Cover from "$lib/components/Cover.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import Status from "$lib/components/Status.svelte";
  import Tile from "$lib/components/Tile.svelte";
  import TrackTable from "$lib/components/TrackTable.svelte";

  type Filter = "Playlists" | "Albums" | "Artists" | "Songs";
  const sections: Record<Filter, LibrarySection> = { Playlists: "playlists", Albums: "albums", Artists: "artists", Songs: "liked" };
  let filter = $state<Filter>("Playlists");
  let sort = $state<"recent" | "az">("recent");
  let layout = $state<"grid" | "list">((localStorage.getItem("libLayout") as "grid" | "list") ?? "grid");
  $effect(() => localStorage.setItem("libLayout", layout));

  let items = $state<ShelfItem[]>([]);
  let cursor = $state<string | null>(null);
  const page = new Resource<LibraryPage>(async () => {
    const p = await call((a) => a.library(sections[filter]));
    catalog.addItems(p.items);
    items = p.items;
    cursor = p.cursor;
    return p;
  }, false);
  $effect(() => {
    filter;
    account.state.state;
    void page.reload();
  });
  // Keep loading pages so "Songs" and long lists are complete.
  let loadingMore = $state(false);
  $effect(() => {
    if (cursor && !loadingMore) void more();
  });
  async function more() {
    if (!cursor || items.length > 5000) return;
    loadingMore = true;
    const sec = sections[filter];
    try {
      const p = await call((a) => a.libraryMore(sec, cursor!));
      if (sec !== sections[filter]) return;
      catalog.addItems(p.items);
      items = [...items, ...p.items];
      cursor = p.cursor;
    } catch {
      cursor = null;
    } finally {
      loadingMore = false;
    }
  }

  const sorted = $derived(sort === "az" ? [...items].sort((a, b) => ("title" in a ? a.title : "name" in a ? a.name : "").localeCompare("title" in b ? b.title : "name" in b ? b.name : "")) : items);
  const cols = $derived(sorted.filter((i) => i.type === "collection") as (CollectionCard & { type: "collection" })[]);
  const songs = $derived(items.filter((i) => i.type === "track") as Track[]);
</script>

<div class="page">
  <h1 class="page-title">Library</h1>

  <div class="bar">
    <div class="chips" role="group" aria-label="Show">
      {#each ["Playlists", "Albums", "Artists", "Songs"] as const as f}
        <button class="chip" aria-pressed={filter === f} onclick={() => (filter = f)}>{f}</button>
      {/each}
    </div>
    {#if filter !== "Songs"}
      <div class="tools">
        <label class="sort">
          <span class="sr">Sort by</span>
          <select bind:value={sort}>
            <option value="recent">Recently added</option>
            <option value="az">A to Z</option>
          </select>
        </label>
        <div class="seg" role="group" aria-label="Layout">
          <button aria-pressed={layout === "grid"} aria-label="Grid" onclick={() => (layout = "grid")}><Icon name="grid" size={15} /></button>
          <button aria-pressed={layout === "list"} aria-label="List" onclick={() => (layout = "list")}><Icon name="list" size={15} /></button>
        </div>
      </div>
    {/if}
  </div>

  {#if !account.signedIn}
    <Status error={page.error ?? null} empty={page.error ? null : "Sign in to see your playlists, albums, artists and liked songs."} />
    {#if !page.error}<button class="pill primary" onclick={() => nav.go({ name: "settings" })}>Sign in</button>{/if}
  {:else}
    <Status loading={page.loading && !items.length} error={page.error} onretry={() => page.reload()} empty={!page.loading && !items.length ? `No ${filter.toLowerCase()} in your library yet.` : null} />
    {#if filter === "Songs"}
      {#if songs.length}<TrackTable tracks={songs} label="Liked music" collectionId="ytm:VLLM" />{/if}
    {:else if filter === "Artists"}
      <div class="grid">
        {#each sorted as a (a.type + ("id" in a ? a.id : ""))}
          <Tile item={a} size={0} />
        {/each}
      </div>
    {:else if layout === "grid"}
      <div class="grid">
        {#if filter === "Playlists"}
          <button class="new" onclick={() => ui.say("Create playlists on music.youtube.com for now; they show up here")}>
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
    {#if loadingMore}<p class="loading">Loading more…</p>{/if}
  {/if}
</div>

<style>
  .bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin: 16px 0 24px;
    flex-wrap: wrap;
  }
  .chips {
    display: flex;
    gap: 8px;
  }
  .tools {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
  }
  select {
    height: 30px;
    padding: 0 10px;
    border-radius: 8px;
    border: 1px solid var(--line);
    background: var(--panel);
    color: var(--text);
    font: inherit;
    font-size: 13px;
  }
  .seg {
    display: flex;
    padding: 2px;
    border-radius: 8px;
    border: 1px solid var(--line);
  }
  .seg button {
    width: 30px;
    height: 26px;
    display: grid;
    place-items: center;
    border-radius: 6px;
    color: var(--mute);
  }
  .seg button[aria-pressed="true"] {
    background: var(--hover);
    color: var(--text);
  }
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
  .loading {
    margin-top: 16px;
    color: var(--mute);
    font-size: 13px;
  }
</style>
