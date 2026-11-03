<script lang="ts">
  // Nothing playing yet: how to start.
  import { nav, ui } from "$lib/state.svelte";
  import Icon from "../Icon.svelte";
</script>

<div class="idle-card">
  <div class="idle-art" aria-hidden="true"><Icon name="lyrics" size={34} /></div>
  <h2>Nothing playing yet</h2>
  <p>Play a song and it shows up here, with its lyrics and what's coming next.</p>
  <div class="idle-actions">
    <button class="pill primary" onclick={() => (ui.palette = true)}><Icon name="search" size={15} /> Search</button>
    <button class="pill" onclick={() => nav.go({ name: "explore" })}><Icon name="compass" size={15} /> Explore</button>
  </div>
</div>

<style>
  .idle-card h2 {
    font-family: var(--font-display);
    font-size: 22px;
    font-weight: 650;
    line-height: 1.15;
    letter-spacing: -0.02em;
    text-wrap: balance;
  }
  .idle-card {
    display: grid;
    justify-items: center;
    gap: 10px;
    padding: 36px 20px 28px;
    border-radius: 14px;
    text-align: center;
    background: color-mix(in srgb, var(--text) 4%, transparent);
    border: 1px dashed color-mix(in srgb, var(--text) 14%, transparent);
  }
  .idle-art {
    display: grid;
    place-items: center;
    width: 72px;
    height: 72px;
    margin-bottom: 4px;
    border-radius: 18px;
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .idle-card h2 {
    margin: 0;
    font-size: 17px;
  }
  .idle-card p {
    margin: 0;
    max-width: 260px;
    color: var(--mute);
    font-size: 13px;
    line-height: 1.45;
    text-wrap: pretty;
  }
  .idle-actions {
    display: flex;
    gap: 8px;
    margin-top: 8px;
  }
  .idle-actions .pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
</style>
