<script lang="ts">
  // Lane D's overlays, mounted once for the whole app: the "Undo" offer
  // after "Don't play this again" (D36), where the app's toasts appear. A
  // newer toast covers it.
  import { taste, ui } from "$lib/state.svelte";
  import Icon from "$lib/components/Icon.svelte";

  const u = $derived(taste.undo && (!ui.toast || ui.toast.id < taste.undo.id) ? taste.undo : null);
</script>

{#if u}
  {#key u.id}
    <div class="undo" role="status">
      <Icon name="check" size={16} />
      <span>{u.text}</span>
      <button onclick={() => void taste.unblock(u.kind, u.target)}>Undo</button>
      <button class="x" aria-label="Dismiss" onclick={() => (taste.undo = null)}><Icon name="x" size={14} /></button>
    </div>
  {/key}
{/if}

<style>
  .undo {
    position: fixed;
    left: 50%;
    bottom: calc(var(--deck-h) + 16px);
    transform: translateX(-50%);
    z-index: 70;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 6px 5px 12px;
    border-radius: 99px;
    background: var(--text);
    color: var(--ink);
    font-weight: 600;
    font-size: 13.5px;
    box-shadow: var(--shadow);
    max-width: min(560px, calc(100vw - 48px));
    animation: rise 0.25s var(--ease);
  }
  span {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  button {
    flex: none;
    height: 28px;
    padding: 0 12px;
    border-radius: 99px;
    font-weight: 700;
    color: var(--ink);
    background: color-mix(in srgb, var(--ink) 12%, transparent);
  }
  button:hover {
    background: color-mix(in srgb, var(--ink) 20%, transparent);
  }
  .x {
    display: grid;
    place-items: center;
    width: 28px;
    padding: 0;
    background: none;
    opacity: 0.7;
  }
  @keyframes rise {
    from {
      opacity: 0;
      transform: translate(-50%, 8px);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .undo {
      animation: none;
    }
  }
</style>
