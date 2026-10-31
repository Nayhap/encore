<script lang="ts">
  import { downloads, KEEP_OFFLINE, needsOnline, ui, type MenuItem } from "$lib/state.svelte";
  import Icon from "./Icon.svelte";

  // Download a playlist, album or liked songs, and keep it up to date (D23).
  // Once kept it says how far it got and opens a menu instead of toggling,
  // so a stray click never deletes anything. "Downloaded" and its check
  // mark only once every song is on this computer.
  let { id }: { id: string } = $props();

  const k = $derived(downloads.keptFor(id));
  const reading = $derived(!!k && !k.songs && (downloads.syncing(id) || k.synced == null));
  const pending = $derived(!!k && k.ready + k.failed < k.songs);
  const complete = $derived(!!k && k.songs > 0 && k.ready === k.songs);
  const fraction = $derived(k && k.songs ? k.ready / k.songs : 0);
  const label = $derived.by(() => {
    if (!k) return "Download";
    if (reading) return "Reading the list…";
    if (complete) return "Downloaded";
    if (pending) return k.paused ? `Paused · ${k.ready} of ${k.songs}` : `Downloading ${k.ready} of ${k.songs}`;
    return `${k.ready} of ${k.songs} downloaded`;
  });

  function menu(e: MouseEvent) {
    if (!k) return;
    const items: MenuItem[] = [
      { label: "Check for changes", icon: "repeat", disabled: needsOnline("check for changes") ?? (k.mine ? undefined : "Only the account that downloaded it can check it for changes."), run: () => downloads.sync(k.card.id) },
      ...(k.failed ? [{ label: "Try failed songs again", icon: "download", run: () => downloads.retry() }] : []),
      { label: k.paused ? "Resume downloading" : "Pause downloading", icon: k.paused ? "play" : "pause", run: () => downloads.pauseKept(id, !k.paused, k) },
      { label: "Remove download…", icon: "trash", danger: true, run: () => downloads.unkeep(id, k) },
    ];
    ui.openMenu(e, items);
  }
</script>

{#if downloads.available}
  {#if k}
    <button class="pill kept" class:partial={!complete && !pending && !reading} aria-haspopup="menu" onclick={menu}>
      {#if pending || reading}
        <span class="ring" class:spin={reading} role="progressbar" aria-label="Downloaded so far" aria-valuemin={0} aria-valuemax={k.songs} aria-valuenow={k.ready} style:--p={reading ? 0.3 : fraction}></span>
      {:else if complete}
        <Icon name="downloaded" size={16} />
      {:else}
        <Icon name="alert" size={16} />
      {/if}
      {label}
      <Icon name="down" size={14} />
    </button>
  {:else}
    {@const why = ui.online ? undefined : KEEP_OFFLINE}
    <button class="pill" aria-disabled={!!why} title={why} onclick={() => (why ? ui.say(why) : void downloads.keep(id))}>
      <Icon name="download" size={16} />Download
    </button>
  {/if}
{/if}

<style>
  .kept {
    color: var(--accent);
  }
  .kept.partial {
    color: var(--text);
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
  .ring.spin {
    animation: spin 0.9s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
