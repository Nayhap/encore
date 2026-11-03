<script lang="ts">
  // The wash of colour behind the now playing panel: the cover's two hues,
  // and over them the cover itself, blurred (D27).
  import CoverWash from "../CoverWash.svelte";
</script>

<div class="ambient" aria-hidden="true">
  <CoverWash />
</div>

<style>
  .ambient {
    position: absolute;
    inset: 0;
    background:
      radial-gradient(120% 60% at 0% 0%, var(--tint-a), transparent 70%),
      radial-gradient(100% 50% at 100% 30%, var(--tint-b), transparent 70%);
    pointer-events: none;
  }
</style>
