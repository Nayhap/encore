<script lang="ts">
  // The wash of colour behind the now playing panel (lane E: the cover
  // ambience of D27 grows from here).
</script>

<div class="ambient" aria-hidden="true"></div>

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
