<script lang="ts">
  import type { LyricLine } from "$lib/backend";
  import { fmt, lyricSegments, type Segment } from "$lib/data";
  import Icon from "./Icon.svelte";

  // The seek bar, drawn from the song's synced lyrics: one segment per sung
  // line, gaps where it is instrumental, taller segments for lines that repeat.
  // With `onloop`, it also shows and edits the A–B loop (D26): a lit band
  // with a handle at each end, a pill with its times and ✕, and Shift-drag
  // to draw a new one.
  type Span = { a: number; b: number };
  let {
    lines,
    duration,
    position,
    onseek,
    size = "deck",
    loop = null,
    mark = null,
    onloop,
    onclearloop,
  }: {
    lines: LyricLine[] | null;
    duration: number;
    position: number;
    onseek: (t: number) => void;
    size?: "deck" | "large";
    loop?: Span | null;
    /** A loop's start, waiting for its end (the A key). */
    mark?: number | null;
    onloop?: (a: number, b: number) => void;
    onclearloop?: () => void;
  } = $props();

  /** The shortest loop (D26's MIN_LOOP). */
  const MIN = 1;

  const track = $derived({ dur: duration > 0 ? duration : 1 });
  const segs = $derived(lyricSegments(lines, duration));
  let el: HTMLDivElement;
  let hover = $state<{ x: number; t: number } | null>(null);
  let drag = $state<number | null>(null);
  // A loop being drawn (Shift-drag) or one of its ends being moved.
  let drawing = $state<{ from: number; to: number } | null>(null);
  let moving = $state<{ end: "a" | "b"; a: number; b: number } | null>(null);

  const shown = $derived(drag ?? position);
  const pct = (t: number) => `${(t / track.dur) * 100}%`;
  const band = $derived.by<Span | null>(() => {
    if (drawing) return { a: Math.min(drawing.from, drawing.to), b: Math.max(drawing.from, drawing.to) };
    if (moving) return { a: moving.a, b: moving.b };
    return loop;
  });

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
    if (e.shiftKey && onloop) {
      const t = tAt(e.clientX);
      drawing = { from: t, to: t };
      return;
    }
    drag = tAt(e.clientX);
  }
  function move(e: PointerEvent) {
    const t = tAt(e.clientX);
    const w = el.getBoundingClientRect().width;
    // Keep the tooltip inside the bar's ends so it never runs off-window.
    hover = { x: Math.min(Math.max(e.clientX - el.getBoundingClientRect().left, Math.min(120, w / 2)), w - Math.min(120, w / 2)), t };
    if (drawing) drawing = { ...drawing, to: t };
    else if (drag !== null) drag = t;
  }
  function up() {
    if (drawing) {
      const a = Math.min(drawing.from, drawing.to);
      const b = Math.max(drawing.from, drawing.to);
      drawing = null;
      // A click with Shift held isn't a loop; a short drag isn't either.
      if (b - a >= MIN) onloop?.(a, b);
      return;
    }
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

  // The loop's ends: drag them, or focus one and use the arrow keys.
  function grab(end: "a" | "b", e: PointerEvent) {
    if (!loop) return;
    e.stopPropagation();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    moving = { end, a: loop.a, b: loop.b };
  }
  function slide(e: PointerEvent) {
    if (!moving) return;
    const t = tAt(e.clientX);
    moving = moving.end === "a" ? { ...moving, a: Math.min(Math.max(0, t), moving.b - MIN) } : { ...moving, b: Math.max(Math.min(track.dur, t), moving.a + MIN) };
  }
  function drop() {
    if (!moving) return;
    const { a, b } = moving;
    moving = null;
    if (loop && (Math.abs(a - loop.a) > 0.05 || Math.abs(b - loop.b) > 0.05)) onloop?.(a, b);
  }
  function nudge(end: "a" | "b", e: KeyboardEvent) {
    if (!loop) return;
    const d = e.key === "ArrowLeft" ? -1 : e.key === "ArrowRight" ? 1 : 0;
    if (!d) return;
    e.preventDefault();
    e.stopPropagation();
    const step = d * (e.shiftKey ? 5 : 1);
    if (end === "a") onloop?.(Math.min(Math.max(0, loop.a + step), loop.b - MIN), loop.b);
    else onloop?.(loop.a, Math.max(Math.min(track.dur, loop.b + step), loop.a + MIN));
  }

  const hoverLine = $derived(hover ? lineAt(hover.t) : null);
</script>

<div class="songmap {size}">
  <div class="track">
    <div
      class="map {size}"
      class:has-lyrics={!!segs}
      class:dragging={drag !== null}
      class:drawing={!!drawing}
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
      {#if band}
        <div class="band" class:live={!!drawing || !!moving} style:left={pct(band.a)} style:width={pct(band.b - band.a)}></div>
      {/if}
      <div class="rail"></div>
      {#if segs}
        {#each segs as s (s.start)}
          {@const fill = Math.min(Math.max((shown - s.start) / (s.end - s.start), 0), 1)}
          <div class="seg {s.tier}" class:lit={hoverLine === s} style:left={pct(s.start)} style:width={pct(s.end - s.start)}>
            <i style:transform="scaleX({fill})"></i>
          </div>
        {/each}
        <div class="played" style:transform="scaleX({shown / track.dur})"></div>
      {:else}
        <div class="seg plain" style:left="0" style:width="100%">
          <i style:transform="scaleX({shown / track.dur})"></i>
        </div>
      {/if}
      {#if mark != null && !band}
        <div class="mark" style:left={pct(mark)} title="Loop from {fmt(mark)}: press A again where it should end"></div>
      {/if}
      <!-- The playhead rides a full-width carriage moved by transform, so
           playback never lays the bar out again (D17). -->
      <div class="carriage" style:transform="translateX({(shown / track.dur) * 100}%)"><div class="head"></div></div>

      {#if hover}
        <div class="tip" style:left="{hover.x}px">
          <span class="mono">{fmt(hover.t)}</span>
          {#if drawing}
            <span class="line">Loop {fmt(band?.a)}–{fmt(band?.b)}</span>
          {:else if segs}
            <span class="line">{hoverLine ? hoverLine.text : "Instrumental"}</span>
          {/if}
        </div>
      {/if}
    </div>

    {#if loop && onloop}
      {@const b = band ?? loop}
      {#each ["a", "b"] as const as end (end)}
        <button
          class="grip {end}"
          style:left={pct(b[end])}
          role="slider"
          aria-label={end === "a" ? "Loop start" : "Loop end"}
          aria-valuemin={0}
          aria-valuemax={Math.round(track.dur)}
          aria-valuenow={Math.round(b[end])}
          aria-valuetext={fmt(b[end])}
          title="{end === 'a' ? 'Loop starts' : 'Loop ends'} at {fmt(b[end])}: drag to move"
          onpointerdown={(e) => grab(end, e)}
          onpointermove={slide}
          onpointerup={drop}
          onkeydown={(e) => nudge(end, e)}
        ><i></i></button>
      {/each}
      {#if !moving}
        <button class="unloop" style:left={pct(b.b)} aria-label="Clear the loop {fmt(b.a)}–{fmt(b.b)}" title="Clear the loop (A)" onclick={() => onclearloop?.()}><Icon name="x" size={10} /></button>
      {/if}
    {/if}
  </div>

</div>

<style>
  .songmap {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    container-type: inline-size;
  }
  .track {
    position: relative;
    flex: 1;
    min-width: 0;
  }
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
  .map.drawing {
    cursor: col-resize;
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
    width: 100%;
    transform-origin: left;
    will-change: transform;
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
  .carriage {
    position: absolute;
    inset: 0;
    pointer-events: none;
    will-change: transform;
  }
  .head {
    position: absolute;
    left: 0;
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

  /* The A–B loop: a lit stretch behind the bar, its ends as handles. */
  .band {
    position: absolute;
    top: 1px;
    bottom: 1px;
    border-radius: 5px;
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent) 38%, transparent);
    pointer-events: none;
  }
  .band.live {
    background: color-mix(in srgb, var(--accent) 24%, transparent);
  }
  .mark {
    position: absolute;
    top: 2px;
    bottom: 2px;
    width: 2px;
    margin-left: -1px;
    border-radius: 2px;
    background: var(--accent);
    pointer-events: none;
  }
  .mark::before {
    content: "";
    position: absolute;
    top: -3px;
    left: -2px;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent);
  }
  .grip {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 14px;
    margin-left: -7px;
    display: grid;
    place-items: center;
    cursor: ew-resize;
    touch-action: none;
    z-index: 2;
  }
  .grip i {
    width: 4px;
    height: 100%;
    border-radius: 3px;
    background: var(--accent);
    box-shadow: 0 0 0 2px var(--panel);
    transition: transform 0.15s var(--ease);
  }
  .grip:hover i,
  .grip:focus-visible i {
    transform: scaleX(1.4);
  }
  .grip:focus-visible {
    outline: none;
  }
  .grip:focus-visible i {
    box-shadow: 0 0 0 2px var(--panel), 0 0 0 4px color-mix(in srgb, var(--accent) 50%, transparent);
  }

  /* Clears the loop: a small mark riding the band's end. */
  .unloop {
    position: absolute;
    top: -9px;
    width: 16px;
    height: 16px;
    margin-left: 3px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    color: var(--accent-ink);
    background: var(--accent);
    box-shadow: 0 0 0 2px var(--panel);
    opacity: 0.85;
    z-index: 3;
    transition: opacity 0.15s, transform 0.15s var(--ease);
  }
  .unloop:hover,
  .unloop:focus-visible {
    opacity: 1;
    transform: scale(1.12);
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
