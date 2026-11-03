<script lang="ts">
  // An artist's Wikipedia summary, for artists YouTube gives no description
  // (D29), with its attribution and a link to the article.
  import { openExternal, wikiFor } from "$lib/state.svelte";

  let { id, name }: { id: string; name: string } = $props();
  const w = $derived(wikiFor(id, name));
  let open = $state(false);
</script>

{#if w}
  <div class="wiki">
    <p class="text" class:open>{w.extract}</p>
    <p class="src">
      {#if !open && w.extract.length > 260}<button class="more" onclick={() => (open = true)}>More</button> ·{/if}
      From <button onclick={() => openExternal(w.url)}>Wikipedia</button>, under
      <button onclick={() => openExternal("https://creativecommons.org/licenses/by-sa/4.0/")}>CC BY-SA 4.0</button>
    </p>
  </div>
{/if}

<style>
  .wiki {
    max-width: 56ch;
    display: grid;
    gap: 4px;
    animation: in 0.4s var(--ease);
  }
  .text {
    color: var(--mute);
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    overflow: hidden;
  }
  .text.open {
    display: block;
  }
  .src {
    font-size: 12px;
    color: var(--dim);
  }
  .src button {
    color: var(--mute);
    font-weight: 550;
  }
  .src button:hover {
    color: var(--text);
    text-decoration: underline;
    text-underline-offset: 3px;
  }
  .src .more {
    color: var(--text);
  }
  @keyframes in {
    from {
      opacity: 0;
    }
  }
</style>
