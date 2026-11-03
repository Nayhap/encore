<script lang="ts">
  // Library → Songs: your liked songs.
  import type { Track } from "$lib/backend";
  import TrackTable from "$lib/components/TrackTable.svelte";

  let { songs }: { songs: Track[] } = $props();
</script>

<TrackTable tracks={songs} label="Liked music" collectionId="ytm:VLLM" />
