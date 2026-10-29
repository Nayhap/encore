<script lang="ts">
  import type { LyricLine } from "$lib/backend";
  import { fmt, lyricSegments, type Segment } from "$lib/data";

  // The seek bar, drawn from the song's synced lyrics: one segment per sung
  // line, gaps where it is instrumental, taller segments for lines that repeat.
  let {
    lines,
    duration,
    position,
    onseek,
    size = "deck",
  }: { lines: LyricLine[] | null; duration: number; position: number; onseek: (t: number) => void; size?: "deck" | "large" } = $props();

  const track = $derived({ dur: duration > 0 ? duration : 1 });
  const segs = $derived(lyricSegments(lines, duration));
  let el: HTMLDivElement;
  let hover = $state<{ x: number; t: number } | null>(null);
  let drag = $state<number | null>(null);

  const shown = $derived(drag ?? position);
  const pct = (t: number) => `${(t / track.dur) * 100}%`;

  const tAt = (clientX: number) => {
    const r = el.getBoundingClientRect();
    return Math.min(Math.max((clientX - r.left) / r.width, 0), 1) * track.dur;
  };

  const lineAt = (t: number): Segment | null => {
    if (!segs) return null;
    let found: Segment | null = null;
    for (const s of segs) if (s.start <= t) found = s;
    // Past the end of a line and well into a gap reads as instrumental.
    if (found && t > found.end + 1.5 && found.next - found.end > 6) return null;
    return found;
  };

  function down(e: PointerEvent) {
    el.setPointerCapture(e.pointerId);
    drag = tAt(e.clientX);
  }
  function move(e: PointerEvent) {
    const t = tAt(e.clientX);
    const w = el.getBoundingClientRect().width;
    // Keep the tooltip inside the bar's ends so it never runs off-window.
    hover = { x: Math.min(Math.max(e.clientX - el.getBoundingClientRect().left, Math.min(120, w / 2)), w - Math.min(120, w / 2)), t };
    if (drag !== null) drag = t;
  }
  function up() {
    if (drag !== null) onseek(drag);
    drag = null;
  }
  function key(e: KeyboardEvent) {
    const step = e.shiftKey ? 15 : 5;
    if (e.key === "ArrowLeft") onseek(position - step);
    else if (e.key === "ArrowRight") onseek(position + step);
    else if (e.key === "Home") onseek(0);
    else return;
    e.preventDefault();
    e.stopPropagation();
  }

  const hoverLine = $derived(hover ? lineAt(hover.t) : null);
</script>

<div
  class="map {size}"
  class:has-lyrics={!!segs}
  class:dragging={drag !== null}
  bind:this={el}
  role="slider"
  tabindex="0"
  aria-label="Seek"
  aria-valuemin={0}
  aria-valuemax={Math.round(track.dur)}
  aria-valuenow={Math.round(shown)}
  aria-valuetext="{fmt(shown)} of {fmt(track.dur)}"
  onpointerdown={down}
  onpointermove={move}
  onpointerup={up}
  onpointerleave={() => (hover = null)}
  onkeydown={key}
>
  <div class="rail"></div>
  {#if segs}
    {#each segs as s (s.start)}
      {@const fill = Math.min(Math.max((shown - s.start) / (s.end - s.start), 0), 1)}
      <div class="seg {s.tier}" class:lit={hoverLine === s} style:left={pct(s.start)} style:width={pct(s.end - s.start)}>
        <i style:transform="scaleX({fill})"></i>
      </div>
    {/each}
    <div class="played" style:width={pct(shown)}></div>
  {:else}
    <div class="seg plain" style:left="0" style:width="100%">
      <i style:transform="scaleX({shown / track.dur})"></i>
    </div>
  {/if}
  <div class="head" style:left={pct(shown)}></div>

  {#if hover}
    <div class="tip" style:left="{hover.x}px">
      <span class="mono">{fmt(hover.t)}</span>
      {#if segs}
        <span class="line">{hoverLine ? hoverLine.text : "Instrumental"}</span>
      {/if}
    </div>
  {/if}
</div>

<style>
  .map {
    position: relative;
    height: 22px;
    flex: 1;
    min-width: 0;
    touch-action: none;
    --h-verse: 4px;
    --h-hook: 7px;
    --h-chorus: 11px;
  }
  .map.large {
    height: 30px;
    --h-verse: 5px;
    --h-hook: 10px;
    --h-chorus: 15px;
  }
  .rail,
  .played {
    position: absolute;
    left: 0;
    top: 50%;
    height: 2px;
    margin-top: -1px;
    border-radius: 2px;
  }
  .rail {
    right: 0;
    background: color-mix(in srgb, var(--text) 10%, transparent);
  }
  .played {
    background: color-mix(in srgb, var(--accent) 55%, transparent);
  }
  .seg {
    position: absolute;
    top: 50%;
    height: var(--h-verse);
    transform: translateY(-50%);
    border-radius: 2px;
    overflow: hidden;
    background: color-mix(in srgb, var(--text) 26%, transparent);
    transition: height 0.18s var(--ease), background-color 0.18s;
    min-width: 3px;
  }
  .seg.hook {
    height: var(--h-hook);
    background: color-mix(in srgb, var(--accent) 34%, transparent);
  }
  .seg.chorus {
    height: var(--h-chorus);
    background: color-mix(in srgb, var(--accent) 46%, transparent);
  }
  .seg.plain {
    height: 4px;
  }
  .seg i {
    position: absolute;
    inset: 0;
    background: var(--accent);
    transform-origin: left;
  }
  .seg.lit {
    background: color-mix(in srgb, var(--text) 55%, transparent);
  }
  .map:hover .seg.plain,
  .map:focus-visible .seg.plain {
    height: 6px;
  }
  .head {
    position: absolute;
    top: 50%;
    width: 3px;
    height: calc(var(--h-chorus) + 8px);
    margin-left: -1.5px;
    transform: translateY(-50%);
    border-radius: 3px;
    background: var(--text);
    box-shadow: 0 0 0 2px var(--ink);
    opacity: 0;
    transition: opacity 0.15s;
  }
  .map:not(.has-lyrics) .head {
    height: 12px;
  }
  .map:hover .head,
  .map:focus-visible .head,
  .dragging .head,
  .has-lyrics .head {
    opacity: 1;
  }
  .map:focus-visible {
    outline-offset: 4px;
  }

  .tip {
    position: absolute;
    bottom: calc(100% + 8px);
    transform: translateX(-50%);
    display: flex;
    align-items: baseline;
    gap: 8px;
    max-width: 340px;
    padding: 6px 10px;
    border-radius: 8px;
    background: var(--raise);
    box-shadow: var(--shadow);
    font-size: 12.5px;
    white-space: nowrap;
    pointer-events: none;
    z-index: 5;
  }
  .tip .mono {
    color: var(--mute);
    font-size: 11.5px;
  }
  .tip .line {
    overflow: hidden;
    text-overflow: ellipsis;
    font-weight: 550;
  }
</style>
