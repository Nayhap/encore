<script lang="ts">
  // The cover ambience (D27): the playing song's cover, small and blurred
  // into a wash. Used behind the now playing panel and full-screen lyrics.
  // Covers crossfade from one song to the next; Linux shows nothing here
  // (its gradients stay, D17).
  import { fade } from "svelte/transition";
  import { ambience, washFade } from "$lib/state.svelte";
  import Cover from "./Cover.svelte";

  let { strength = "panel" }: { strength?: "panel" | "theater" } = $props();
  const art = $derived(ambience.art);
  const key = $derived(art && "key" in art ? art.key : art && "seed" in art ? `seed:${art.seed}` : ambience.label);
</script>

{#if ambience.on}
  <div class="wash {strength}" aria-hidden="true">
    {#key key}
      <div class="layer" transition:fade={{ duration: washFade() }}>
        <Cover {art} radius={0} size={120} label={ambience.label} />
      </div>
    {/key}
  </div>
{/if}

<style>
  .wash {
    position: absolute;
    inset: 0;
    overflow: hidden;
    pointer-events: none;
  }
  .layer {
    position: absolute;
    inset: 0;
  }
  .layer :global(> *) {
    position: absolute;
    left: 50%;
    width: 140%;
    height: auto;
    aspect-ratio: 1;
    transform: translateX(-50%);
    filter: blur(var(--wash-blur)) saturate(1.6);
    opacity: var(--wash-alpha);
  }
  /* The panel: the cover rises behind its card and fades out before the queue. */
  .panel {
    --wash-blur: 44px;
    mask-image: linear-gradient(#000 0%, #000 22%, transparent 68%);
  }
  .panel .layer :global(> *) {
    top: -30%;
  }
  /* Full-screen lyrics: the whole room takes the cover's light. */
  .theater {
    --wash-blur: 90px;
  }
  .theater .layer :global(> *) {
    top: 50%;
    width: 120vmax;
    transform: translate(-50%, -50%);
    /* Under lyrics: enough to colour the room, never enough to wash out text. */
    opacity: calc(var(--wash-alpha) * 0.55);
  }
  :global(:root[data-os="linux"]) .wash {
    display: none;
  }
</style>
