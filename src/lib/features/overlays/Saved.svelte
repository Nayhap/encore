<script lang="ts">
  // Lane C's sheets, mounted once above the views: naming a saved queue or
  // a folder (`saved.naming`).
  import { saved } from "$lib/state.svelte";
  import NameSheet from "$lib/components/saved/NameSheet.svelte";
</script>

{#if saved.naming}
  {#key saved.naming}<NameSheet sheet={saved.naming} />{/key}
{/if}
