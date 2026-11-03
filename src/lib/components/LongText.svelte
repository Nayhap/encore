<script lang="ts">
  // A description that could be any length (an album's, a playlist's, an
  // artist's): three lines, with More only when it really runs past them,
  // and Less to fold it again. YouTube's trailing "From Wikipedia (…) under
  // … (…)" credit becomes a short line with real links.
  import type { Snippet } from "svelte";
  import { openExternal } from "$lib/state.svelte";
  import { splitAttribution } from "$lib/text";

  let { text, lines = 3, credit: extra }: { text: string; lines?: number; credit?: Snippet } = $props();

  const parts = $derived(splitAttribution(text.trim()));
  let open = $state(false);
  let long = $state(false);
  let el = $state<HTMLParagraphElement>();

  // Measured, not guessed from the length: the same text is three lines in a
  // wide window and eight in a narrow one.
  $effect(() => {
    if (!el) return;
    const node = el;
    const check = () => {
      if (!open) long = node.scrollHeight > node.clientHeight + 1;
    };
    check();
    const ro = new ResizeObserver(check);
    ro.observe(node);
    return () => ro.disconnect();
  });

  // A new text (another page) starts folded.
  $effect(() => {
    void text;
    open = false;
  });
</script>

<div class="long">
  <p bind:this={el} class="text" class:open style:--lines={lines}>{parts.body}</p>
  {#if long || parts.credit || extra}
    <p class="foot">
      {#if long}
        <button class="more" aria-expanded={open} onclick={() => (open = !open)}>{open ? "Less" : "More"}</button>
      {/if}
      {#if parts.credit}
        {#if long}<span aria-hidden="true">·</span>{/if}
        <span>
          From <button class="link" onclick={() => openExternal(parts.credit!.sourceUrl)}>{parts.credit.source}</button>{#if parts.credit.licence},
            under {#if parts.credit.licenceUrl}<button class="link" onclick={() => openExternal(parts.credit!.licenceUrl!)}>{parts.credit.licence}</button>{:else}{parts.credit.licence}{/if}{/if}
        </span>
      {:else if extra}
        {#if long}<span aria-hidden="true">·</span>{/if}
        {@render extra()}
      {/if}
    </p>
  {/if}
</div>

<style>
  .long {
    display: grid;
    gap: 4px;
    max-width: 60ch;
  }
  .text {
    color: var(--mute);
    white-space: pre-line;
    overflow-wrap: anywhere;
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: var(--lines);
    line-clamp: var(--lines);
    overflow: hidden;
  }
  .text.open {
    display: block;
  }
  .foot {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 6px;
    font-size: 12px;
    color: var(--dim);
  }
  .foot button {
    font-weight: 550;
  }
  .more {
    color: var(--text);
  }
  .link {
    color: var(--mute);
  }
  .foot button:hover {
    color: var(--text);
    text-decoration: underline;
    text-underline-offset: 3px;
  }
  .foot :global(button:focus-visible) {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
    border-radius: 3px;
  }
</style>
