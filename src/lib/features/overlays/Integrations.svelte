<script lang="ts">
  // Lane E's sheets (Song details, the Last.fm connect sheet), mounted once
  // for the whole app above the views and under the menus.
  import { lastfm, songDetails } from "$lib/state.svelte";
  import LastfmConnect from "./LastfmConnect.svelte";
  import SongDetails from "./SongDetails.svelte";
</script>

{#if songDetails.track}
  {#key songDetails.track.id}<SongDetails />{/key}
{/if}
{#if lastfm.sheet}
  <LastfmConnect />
{/if}
