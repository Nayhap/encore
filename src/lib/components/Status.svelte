<script lang="ts">
  import type { BackendError } from "$lib/backend";
  import { explain } from "$lib/load.svelte";
  import { nav } from "$lib/state.svelte";

  // Loading, empty and error states shared by the views.
  let { loading = false, error = null, empty = null, onretry }: { loading?: boolean; error?: BackendError | null; empty?: string | null; onretry?: () => void } = $props();
  const info = $derived(error ? explain(error) : null);
</script>

{#if error && info}
  <div class="status" role="alert">
    <b>{info.title}</b>
    <p>{info.hint}</p>
    <div class="acts">
      {#if onretry}<button class="pill" onclick={onretry}>Try again</button>{/if}
      {#if error.code === "authRequired" || error.code === "sessionExpired"}<button class="pill primary" onclick={() => nav.go({ name: "settings" })}>Open account settings</button>{/if}
    </div>
  </div>
{:else if loading}
  <div class="status loading" aria-busy="true">
    <span class="dots" aria-hidden="true"><i></i><i></i><i></i></span>
    <p>Loading…</p>
  </div>
{:else if empty}
  <div class="status"><p>{empty}</p></div>
{/if}

<style>
  .status {
    display: grid;
    gap: 6px;
    justify-items: start;
    padding: 28px 0;
    color: var(--mute);
    max-width: 56ch;
  }
  .status b {
    color: var(--text);
    font-size: 16px;
  }
  .acts {
    display: flex;
    gap: 8px;
    margin-top: 8px;
  }
  .loading {
    grid-auto-flow: column;
    align-items: center;
    justify-content: start;
    gap: 12px;
  }
  .dots {
    display: inline-flex;
    gap: 5px;
  }
  .dots i {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--accent);
    animation: pulse 1s infinite ease-in-out;
  }
  .dots i:nth-child(2) {
    animation-delay: 0.15s;
  }
  .dots i:nth-child(3) {
    animation-delay: 0.3s;
  }
  @keyframes pulse {
    0%,
    100% {
      opacity: 0.25;
    }
    50% {
      opacity: 1;
    }
  }
</style>
