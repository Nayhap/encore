<script lang="ts">
  // The player bar's status chips (`deckChips` in features/hooks): a sleep
  // timer counting down, a private session. Nothing shows until a feature
  // adds one.
  import { deckChips } from "$lib/state.svelte";
  import Icon from "./Icon.svelte";

  const chips = $derived(deckChips.items());
</script>

{#each chips as c (c.key)}
  <button class="deck-chip" title={c.title} aria-label={c.title} onclick={() => c.run?.()}>
    {#if c.icon}<Icon name={c.icon} size={14} />{/if}
    <span class="mono">{c.label}</span>
  </button>
{/each}

<style>
  .deck-chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 26px;
    padding: 0 9px;
    border-radius: 99px;
    font-size: 12px;
    font-weight: 600;
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .deck-chip:hover {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
  }
</style>
