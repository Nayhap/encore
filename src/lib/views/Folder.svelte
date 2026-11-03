<script lang="ts">
  // A folder of playlists and albums (D34, lane C): opened from the
  // sidebar's playlists list as `{ name: "folder", id, title }`.
  import Status from "$lib/components/Status.svelte";

  let { id }: { id: string } = $props();
</script>

<div class="page" data-folder={id}>
  <Status empty="This folder is empty." />
</div>
