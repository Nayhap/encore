<script lang="ts">
  // Every lane's sheets and popovers, one component each, so none of them
  // edits App.svelte (see foundation.md).
  import Playback from "./overlays/Playback.svelte";
  import LibraryTools from "./overlays/LibraryTools.svelte";
  import Saved from "./overlays/Saved.svelte";
  import Taste from "./overlays/Taste.svelte";
  import Integrations from "./overlays/Integrations.svelte";
</script>

<Playback />
<LibraryTools />
<Saved />
<Taste />
<Integrations />
