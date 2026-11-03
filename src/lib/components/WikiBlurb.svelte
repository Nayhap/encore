<script lang="ts">
  // An artist's Wikipedia summary, for artists YouTube gives no description
  // (D29), with its attribution and a link to the article.
  import { openExternal, wikiFor } from "$lib/state.svelte";
  import LongText from "./LongText.svelte";

  let { id, name }: { id: string; name: string } = $props();
  const w = $derived(wikiFor(id, name));
</script>

{#if w}
  <div class="wiki">
    <LongText text={w.extract}>
      {#snippet credit()}
        <span>
          From <button class="link" onclick={() => openExternal(w.url)}>Wikipedia</button>, under
          <button class="link" onclick={() => openExternal("https://creativecommons.org/licenses/by-sa/4.0/")}>CC BY-SA 4.0</button>
        </span>
      {/snippet}
    </LongText>
  </div>
{/if}

<style>
  .wiki {
    max-width: 56ch;
    animation: in 0.4s var(--ease);
  }
  .link {
    color: var(--mute);
    font-weight: 550;
  }
  .link:hover {
    color: var(--text);
    text-decoration: underline;
    text-underline-offset: 3px;
  }
  @keyframes in {
    from {
      opacity: 0;
    }
  }
</style>
