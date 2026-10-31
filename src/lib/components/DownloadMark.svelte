<script lang="ts">
  import { downloads } from "$lib/state.svelte";
  import Icon from "./Icon.svelte";

  // A song's download state, the same everywhere it is listed: a quiet mark
  // once downloaded, a ring filling while it downloads, a clock while it
  // waits, a warning when it failed. The words are in the label and the
  // tooltip, so neither colour nor hover is needed to read it.
  let { id, size = 14 }: { id: string; size?: number } = $props();

  const it = $derived(downloads.items.get(id));
  const pct = $derived(downloads.progress(id));
  const waitText = $derived(downloads.summary?.paused ? "Download paused" : (downloads.summary?.message ?? "Waiting to download"));
</script>

{#if it?.state === "ready"}
  <span class="mark" role="img" aria-label="Downloaded" title="Downloaded"><Icon name="downloaded" {size} /></span>
{:else if it?.state === "downloading"}
  {#if pct != null}
    <span
      class="mark ring"
      role="progressbar"
      aria-label="Downloading"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={Math.round(pct * 100)}
      title="Downloading: {Math.round(pct * 100)}%"
      style:--p={pct}
      style:--s="{size}px"
    ></span>
  {:else}
    <span class="mark ring spin" role="img" aria-label="Downloading" title="Downloading" style:--s="{size}px"></span>
  {/if}
{:else if it?.state === "queued"}
  {@const text = it.error ? `Will try again: ${it.error}` : waitText}
  <span class="mark" role="img" aria-label={text} title={text}><Icon name="clock" {size} /></span>
{:else if it?.state === "failed"}
  {@const text = `Download failed: ${it.error ?? "unknown reason"}`}
  <span class="mark failed" role="img" aria-label={text} title={text}><Icon name="alert" {size} /></span>
{/if}

<style>
  .mark {
    display: inline-grid;
    place-items: center;
    flex: none;
    color: var(--mute);
  }
  .failed {
    color: hsl(0 70% 62%);
  }
  :global([data-theme="light"]) .failed {
    color: hsl(0 65% 46%);
  }
  .ring {
    width: var(--s);
    height: var(--s);
    border-radius: 50%;
    background: conic-gradient(var(--accent) calc(var(--p, 0) * 360deg), color-mix(in srgb, var(--text) 18%, transparent) 0);
    -webkit-mask: radial-gradient(farthest-side, transparent calc(100% - 2.5px), #000 calc(100% - 2px));
    mask: radial-gradient(farthest-side, transparent calc(100% - 2.5px), #000 calc(100% - 2px));
  }
  .ring.spin {
    --p: 0.3;
    animation: spin 0.9s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
