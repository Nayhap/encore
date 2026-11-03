<script lang="ts">
  // The Autoplay switch under the queue (lane A: D33's rule and copy).
  import { player, ui } from "$lib/state.svelte";
</script>

<label class="autoplay">
  <span>
    <b>Autoplay</b>
    <small>{ui.online ? "Keep playing similar songs when the queue ends" : "Waits until you're back online"}</small>
  </span>
  <input type="checkbox" role="switch" checked={player.autoplay} disabled={!ui.online} onchange={(e) => player.setAutoplay(e.currentTarget.checked)} />
</label>

<style>
  .autoplay {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin-top: 12px;
    padding: 12px 0 0;
    border-top: 1px solid var(--line);
  }
  .autoplay span {
    display: grid;
  }
  .autoplay b {
    font-size: 13px;
  }
  .autoplay small {
    font-size: 12px;
    color: var(--mute);
  }
</style>
