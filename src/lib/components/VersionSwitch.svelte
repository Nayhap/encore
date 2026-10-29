<script lang="ts">
  import type { QueueEntry } from "$lib/backend";
  import { lookupVersion, playOtherVersion, versions } from "$lib/state.svelte";

  // Song | Video for the entry that is playing (D21). Shown only once a pair
  // is known; "Video" plays the music video's audio.
  let { entry }: { entry: QueueEntry } = $props();

  const t = $derived(entry.track);
  const pair = $derived(versions.get(t.id));
  let busy = $state(false);

  $effect(() => {
    if (t.kind === "song" || t.kind === "video") void lookupVersion(t);
  });

  async function pick(kind: "song" | "video") {
    if (busy || t.kind === kind) return;
    busy = true;
    try {
      await playOtherVersion(t, entry.uid);
    } finally {
      busy = false;
    }
  }
</script>

{#if pair && (t.kind === "song" || t.kind === "video")}
  <div class="vs" role="radiogroup" aria-label="Version" aria-busy={busy}>
    <button role="radio" aria-checked={t.kind === "song"} disabled={busy} title={t.kind === "song" ? "Playing the song" : "Play the song"} onclick={() => pick("song")}>Song</button>
    <button role="radio" aria-checked={t.kind === "video"} disabled={busy} title={t.kind === "video" ? "Playing the music video's audio" : "Play the music video's audio"} onclick={() => pick("video")}>Video</button>
  </div>
{/if}

<style>
  .vs {
    display: inline-flex;
    flex: none;
    padding: 2px;
    gap: 2px;
    border-radius: 99px;
    background: var(--hover);
    box-shadow: inset 0 0 0 1px var(--line);
  }
  button {
    height: 24px;
    padding: 0 10px;
    border-radius: 99px;
    font-size: 12px;
    font-weight: 600;
    color: var(--mute);
    transition: background-color 0.15s, color 0.15s;
  }
  button:hover:not([aria-checked="true"]) {
    color: var(--text);
  }
  button[aria-checked="true"] {
    background: var(--text);
    color: var(--ink);
  }
  button:disabled {
    opacity: 0.6;
  }
  .vs[aria-busy="true"] button[aria-checked="false"] {
    animation: pulse 1s ease-in-out infinite alternate;
  }
  @keyframes pulse {
    to {
      opacity: 0.35;
    }
  }
</style>
