<script lang="ts">
  // Lane B's sheets and popovers (the multi-playlist picker, duplicate review), mounted once for the
  // whole app above the views and under the menus. Show them from this
  // lane's state module; nothing here until it has one.
  import { playlistPicker } from "$lib/state.svelte";
  import PlaylistPicker from "$lib/components/library-tools/PlaylistPicker.svelte";
</script>

{#if playlistPicker.open}
  {#key playlistPicker.trackIds}<PlaylistPicker />{/key}
{/if}
