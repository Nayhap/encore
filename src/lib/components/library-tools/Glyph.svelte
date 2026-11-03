<script lang="ts" module>
  // The library tools' own icons, drawn like Icon.svelte's (24 px grid,
  // round 1.75 strokes): sort, select, duplicates.
  const paths: Record<string, string> = {
    sort: "M4 7h9M4 12h6M4 17h4M17 5v14m0 0-3-3m3 3 3-3",
    sortUp: "M4 7h4M4 12h6M4 17h9M17 19V5m0 0-3 3m3-3 3 3",
    select: "M5.5 4h13A1.5 1.5 0 0 1 20 5.5v13a1.5 1.5 0 0 1-1.5 1.5h-13A1.5 1.5 0 0 1 4 18.5v-13A1.5 1.5 0 0 1 5.5 4Zm3 8.2 2.4 2.4 4.6-5.2",
    dupes: "M8.5 8h10A1.5 1.5 0 0 1 20 9.5v9a1.5 1.5 0 0 1-1.5 1.5h-10A1.5 1.5 0 0 1 7 18.5v-9A1.5 1.5 0 0 1 8.5 8ZM16 8V5.5A1.5 1.5 0 0 0 14.5 4h-9A1.5 1.5 0 0 0 4 5.5v9A1.5 1.5 0 0 0 5.5 16H7",
    unlike: "M12 20s-7.5-4.4-7.5-10.2A4.3 4.3 0 0 1 12 7.2a4.3 4.3 0 0 1 7.5 2.6C19.5 15.6 12 20 12 20ZM4 4l16 16",
  };
</script>

<script lang="ts">
  let { name, size = 18 }: { name: keyof typeof paths | string; size?: number } = $props();
</script>

<svg width={size} height={size} viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d={paths[name] ?? ""} /></svg>

<style>
  svg {
    flex: none;
    display: block;
  }
</style>
