<script lang="ts">
  // The now playing panel. Its parts are components of their own
  // (./stage), so each can change on its own: the card and title, the
  // queue, Autoplay, the ambient wash, the idle card.
  import { player } from "$lib/state.svelte";
  import Ambient from "./stage/Ambient.svelte";
  import Autoplay from "./stage/Autoplay.svelte";
  import Idle from "./stage/Idle.svelte";
  import NowPlaying from "./stage/NowPlaying.svelte";
  import Queue from "./stage/Queue.svelte";

  const tr = $derived(player.current);
</script>

<aside class="stage" aria-label="Now playing">
  {#if tr}
    <Ambient />
    <div class="scroll">
      <NowPlaying {tr} />
      <Queue />
    </div>
  {:else}
    <div class="scroll">
      <Idle />
      <Autoplay />
    </div>
  {/if}
</aside>

<style>
  .stage {
    grid-area: stage;
    position: relative;
    overflow: hidden;
    border-left: 1px solid var(--line);
    background: var(--panel);
  }
  .scroll {
    position: relative;
    height: 100%;
    overflow-y: auto;
    padding: calc(var(--top-h) + 2px) 18px 20px;
  }
</style>
