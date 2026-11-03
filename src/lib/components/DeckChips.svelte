<script lang="ts">
  // The player bar's status chips (`deckChips` in features/hooks): a
  // background job's progress. Nothing shows until a feature adds one. A
  // chip that opens a menu opens it where it was clicked (`ui.openMenuAgain`).
  import { deckChips, ui } from "$lib/state.svelte";
  import Icon from "./Icon.svelte";

  const chips = $derived(deckChips.items());
</script>

{#each chips as c (c.key)}
  <button
    class="deck-chip"
    title={c.title}
    aria-label={c.title}
    onclick={(e) => {
      const r = e.currentTarget.getBoundingClientRect();
      ui.lastMenuAt = e.clientX || e.clientY ? { x: e.clientX, y: e.clientY } : { x: r.left, y: r.top };
      c.run?.();
    }}
  >
    {#if c.icon}<Icon name={c.icon} size={14} />{/if}
    <span class="num">{c.label}</span>
  </button>
{/each}

<style>
  /* Quiet like the session control: a background job's progress, not an alarm. */
  .deck-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 10px 0 9px;
    border-radius: 99px;
    font-size: 12px;
    font-weight: 600;
    color: var(--text);
    border: 1px solid var(--line);
    white-space: nowrap;
  }
  .deck-chip :global(svg) {
    color: var(--accent);
  }
  .deck-chip .num {
    font-variant-numeric: tabular-nums;
  }
  .deck-chip:hover {
    background: var(--hover);
  }
</style>
