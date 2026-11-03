<script lang="ts">
  // A folder of playlists and albums (D34): opened from the sidebar as
  // `{ name: "folder", id, title }`. Folders are Encore's own, kept on this
  // computer; what's in them opens and plays as usual.
  import { catalog } from "$lib/catalog.svelte";
  import { chooseCover, folderMenu, renameFolder, saved, ui } from "$lib/state.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import Status from "$lib/components/Status.svelte";
  import Tile from "$lib/components/Tile.svelte";
  import FolderCover from "$lib/components/saved/FolderCover.svelte";

  let { id }: { id: string } = $props();

  const f = $derived(saved.folders.find((x) => x.id === id));
  const albums = $derived(f ? f.items.filter((i) => ["album", "single", "ep"].includes(i.card.kind)).length : 0);
  const lists = $derived(f ? f.items.length - albums : 0);
  const hue = $derived(f ? catalog.hue(f.cover ? { key: f.cover, h1: null, h2: null } : (f.items[0]?.card.art ?? null), f.name)[0] : 200);
  const count = (n: number, one: string, many: string) => (n === 1 ? `1 ${one}` : `${n} ${many}`);
</script>

<div class="page">
  {#if !f}
    <Status loading={!saved.loaded} empty={saved.loaded ? "This folder isn't here any more. It may have been deleted, or it belongs to another account." : null} />
  {:else}
    <header class="hero" style:--ch={hue}>
      <div class="art"><FolderCover folder={f} radius={12} size={544} /></div>
      <div class="info">
        <p class="kind">Folder</p>
        <h1 class:long={f.name.length > 22}>{f.name}</h1>
        <p class="meta">
          {#if lists}<span>{count(lists, "playlist", "playlists")}</span>{/if}
          {#if albums}<span>{count(albums, "album", "albums")}</span>{/if}
          <span>On this computer</span>
        </p>
        <div class="actions">
          <button class="pill" onclick={() => renameFolder(f)}><Icon name="edit" size={15} />Rename</button>
          <button class="pill" onclick={() => chooseCover(f)}><Icon name="sparkle" size={15} />{f.cover ? "Change cover" : "Choose cover"}</button>
          <button class="icon-btn big" aria-label="More actions" onclick={(e) => ui.openMenu(e, folderMenu(f, false))}><Icon name="more" size={19} /></button>
        </div>
      </div>
    </header>

    {#if f.items.length}
      <div class="grid">
        {#each f.items as it (it.itemId)}
          <Tile item={{ type: "collection", ...it.card }} size={0} />
        {/each}
      </div>
    {:else}
      <div class="empty">
        <p>This folder is empty. Drag playlists onto it in the sidebar, or choose <b>Add to folder</b> in a playlist's or album's menu.</p>
      </div>
    {/if}
  {/if}
</div>

<style>
  .hero {
    position: relative;
    display: flex;
    align-items: flex-end;
    gap: 28px;
    padding: 12px 0 28px;
  }
  .hero::before {
    content: "";
    position: absolute;
    inset: -120px -32px 0;
    background: linear-gradient(hsl(var(--ch) 60% 50% / 0.22), transparent);
    z-index: -1;
    pointer-events: none;
  }
  .art {
    width: clamp(160px, 18vw, 232px);
    flex: none;
    box-shadow: 0 24px 50px -20px rgb(0 0 0 / 0.6);
    border-radius: 12px;
  }
  .info {
    min-width: 0;
    display: grid;
    gap: 8px;
  }
  .kind {
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--mute);
  }
  h1 {
    font-family: var(--font-display);
    font-size: clamp(36px, 4.6vw, 64px);
    font-weight: 700;
    line-height: 1;
    letter-spacing: -0.035em;
    font-variation-settings: "wdth" 88;
    text-wrap: balance;
    padding-bottom: 4px;
  }
  h1.long {
    font-size: clamp(28px, 3.2vw, 44px);
  }
  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: 0 6px;
    font-size: 13.5px;
    color: var(--mute);
  }
  .meta > * + *::before {
    content: "·";
    margin-right: 6px;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 10px;
  }
  .actions .pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .big {
    width: 38px;
    height: 38px;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(168px, 1fr));
    gap: 28px 20px;
  }
  .grid :global(.tile) {
    width: auto !important;
  }
  .empty {
    padding: 28px 24px;
    border-radius: 14px;
    border: 1px dashed color-mix(in srgb, var(--text) 16%, transparent);
    color: var(--mute);
    font-size: 13.5px;
    text-align: center;
  }
  .empty b {
    color: var(--text);
    font-weight: 600;
  }
  @media (max-width: 720px) {
    .hero {
      flex-direction: column;
      align-items: flex-start;
    }
  }
</style>
