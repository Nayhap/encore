<script lang="ts">
  import { downloads, needsOnline, ui, type MenuItem } from "$lib/state.svelte";
  import Icon from "./Icon.svelte";

  // Download a playlist, album or liked songs, and keep it up to date (D23).
  // Once kept it reads "Downloaded" (or how far it got) and opens a menu
  // instead of toggling, so a stray click never deletes anything.
  let { id }: { id: string } = $props();

  const k = $derived(downloads.keptFor(id));
  const pending = $derived(!!k && k.ready + k.failed < k.songs);
  const fraction = $derived(k && k.songs ? k.ready / k.songs : 0);
  const label = $derived(!k ? "Download" : k.paused && pending ? `Paused · ${k.ready} of ${k.songs}` : pending ? `Downloading ${k.ready} of ${k.songs}` : "Downloaded");

  function menu(e: MouseEvent) {
    if (!k) return;
    const items: MenuItem[] = [
      { label: "Check for changes", icon: "repeat", disabled: needsOnline("check for changes") ?? (k.mine ? undefined : "Only the account that downloaded it can check it for changes."), run: () => downloads.sync(k.card.id) },
      { label: k.paused ? "Resume downloading" : "Pause downloading", icon: k.paused ? "play" : "pause", run: () => downloads.pauseKept(id, !k.paused) },
      { label: "Remove download…", icon: "trash", danger: true, run: () => downloads.unkeep(id) },
    ];
    ui.openMenu(e, items);
  }
</script>

{#if downloads.available}
  {#if k}
    <button class="pill kept" aria-haspopup="menu" onclick={menu}>
      {#if pending}
        <span class="ring" role="progressbar" aria-label="Downloaded so far" aria-valuemin={0} aria-valuemax={k.songs} aria-valuenow={k.ready} style:--p={fraction}></span>
      {:else}
        <Icon name="downloaded" size={16} />
      {/if}
      {label}
      <Icon name="down" size={14} />
    </button>
  {:else}
    {@const why = needsOnline("download this")}
    <button class="pill" aria-disabled={!!why} title={why} onclick={() => (why ? ui.say(why) : void downloads.keep(id))}>
      <Icon name="download" size={16} />Download
    </button>
  {/if}
{/if}

<style>
  .kept {
    color: var(--accent);
  }
  .pill[aria-disabled="true"] {
    opacity: 0.45;
  }
  .ring {
    width: 15px;
    height: 15px;
    border-radius: 50%;
    background: conic-gradient(currentColor calc(var(--p) * 360deg), color-mix(in srgb, var(--text) 18%, transparent) 0);
    -webkit-mask: radial-gradient(farthest-side, transparent calc(100% - 2.5px), #000 calc(100% - 2px));
    mask: radial-gradient(farthest-side, transparent calc(100% - 2.5px), #000 calc(100% - 2px));
  }
</style>
