<script lang="ts">
  import type { Lyrics } from "$lib/backend";

  // Timed lines follow the clock; plain lyrics (no timing) are shown as text
  // without the karaoke highlight.
  let {
    lyrics,
    duration,
    position,
    onseek,
    size = "card",
    loading = false,
  }: { lyrics: Lyrics | null; duration: number; position: number; onseek: (t: number) => void; size?: "card" | "theater"; loading?: boolean } = $props();

  const track = $derived({ dur: duration || 0, lyrics: lyrics?.lines ?? [] });

  type Row = { t: number; end: number; text: string | null };

  // Gaps of 8s or more become a breathing row, so the reader knows the
  // singer is coming back rather than thinking the lyrics stalled.
  const rows = $derived.by<Row[]>(() => {
    const ls = (track.lyrics ?? []).map((l) => ({ ...l, t: l.t ?? 0 }));
    const out: Row[] = [];
    if (ls.length && ls[0].t >= 8) out.push({ t: 0, end: ls[0].t, text: null });
    ls.forEach((l, i) => {
      const next = ls[i + 1]?.t ?? track.dur;
      const sung = Math.min(next, l.t + Math.max(3.5, l.text.length * 0.13));
      out.push({ t: l.t, end: sung, text: l.text });
      if (next - sung >= 8 && i < ls.length - 1) out.push({ t: sung, end: next, text: null });
    });
    return out;
  });

  const active = $derived.by(() => {
    let a = -1;
    rows.forEach((r, i) => {
      if (r.t <= position) a = i;
    });
    return a;
  });

  let box = $state<HTMLDivElement>();
  let userScrolledAt = -Infinity;

  $effect(() => {
    const i = active;
    if (!box || i < 0) return;
    if (performance.now() - userScrolledAt < 3500) return;
    const el = box.querySelector<HTMLElement>(`[data-i="${i}"]`);
    if (!el) return;
    const target = el.offsetTop - box.clientHeight * (size === "theater" ? 0.38 : 0.34);
    box.scrollTo({ top: target, behavior: "smooth" });
  });

  const progress = (r: Row) => Math.min(Math.max((position - r.t) / (r.end - r.t), 0), 1);
</script>

{#if track.lyrics.length}
  <div
    class="lyrics {size}"
    bind:this={box}
    onwheel={() => (userScrolledAt = performance.now())}
    role="group"
    aria-label="Lyrics"
  >
    {#each rows as r, i (r.t + ":" + i)}
      {#if r.text === null}
        {@const p = i === active ? progress(r) : i < active ? 1 : 0}
        <div class="breath" class:on={i === active} data-i={i} aria-hidden="true">
          {#each [0, 1, 2] as d}
            <span style:opacity={0.25 + Math.min(Math.max(p * 3 - d, 0), 1) * 0.75}></span>
          {/each}
        </div>
      {:else}
        <button
          class="line"
          class:on={i === active}
          class:past={i < active}
          data-i={i}
          style:--p="{(i === active ? progress(r) : i < active ? 1 : 0) * 100}%"
          onclick={() => onseek(r.t)}
        >{r.text}</button>
      {/if}
    {/each}
    {#if lyrics?.attribution || lyrics?.source}<p class="credit">{lyrics?.attribution ?? `Lyrics from ${lyrics?.source}`}</p>{/if}
    <div class="tail"></div>
  </div>
{:else if lyrics?.plain}
  <div class="lyrics plain {size}" role="group" aria-label="Lyrics">
    {#each lyrics.plain.split(/\n/) as l, i (i)}
      {#if l.trim()}<p class="line static">{l}</p>{:else}<div class="gap" aria-hidden="true"></div>{/if}
    {/each}
    <p class="credit">{lyrics.attribution ?? `Lyrics from ${lyrics.source}`} · not synced</p>
    <div class="tail"></div>
  </div>
{:else}
  <div class="none {size}">
    <p>{loading ? "Looking for lyrics…" : "No lyrics for this song."}</p>
    <p class="sub">Lyrics come from YouTube Music and LRCLIB when available.</p>
  </div>
{/if}

<style>
  .credit {
    margin-top: 18px;
    font-size: 12px;
    color: var(--dim);
  }
  .lyrics.plain .line.static {
    cursor: default;
    color: color-mix(in srgb, var(--text) 82%, transparent);
    background: none;
    filter: none;
    transform: none;
  }
  .theater.plain .line.static {
    font-size: clamp(22px, 2.4vw, 32px);
  }
  .lyrics.plain .gap {
    height: 14px;
  }
  @property --p {
    syntax: "<percentage>";
    inherits: false;
    initial-value: 0%;
  }

  .lyrics {
    height: 100%;
    overflow-y: auto;
    scrollbar-width: none;
    mask-image: linear-gradient(transparent, #000 14%, #000 78%, transparent);
    padding: 30% 0 0;
  }
  .lyrics::-webkit-scrollbar {
    display: none;
  }

  .line {
    display: block;
    width: 100%;
    text-align: left;
    font-family: var(--font-display);
    font-weight: 650;
    letter-spacing: -0.012em;
    line-height: 1.22;
    text-wrap: balance;
    padding: 6px 0;
    color: transparent;
    background: linear-gradient(90deg, var(--lit) var(--p), var(--unlit) var(--p));
    -webkit-background-clip: text;
    background-clip: text;
    --lit: color-mix(in srgb, var(--text) 38%, transparent);
    --unlit: color-mix(in srgb, var(--text) 38%, transparent);
    transition: --p 0.12s linear, transform 0.4s var(--ease), filter 0.4s var(--ease);
    transform-origin: left center;
    border-radius: var(--r-sm);
  }
  .line:hover {
    --unlit: color-mix(in srgb, var(--text) 62%, transparent);
    --lit: color-mix(in srgb, var(--text) 62%, transparent);
  }
  .line.on {
    --lit: var(--text);
    --unlit: color-mix(in srgb, var(--text) 45%, transparent);
  }

  .card .line {
    font-size: 19px;
  }
  .theater .line {
    font-size: clamp(28px, 3.3vw, 46px);
    padding: 10px 0;
    font-variation-settings: "wdth" 92;
  }
  .theater .line:not(.on) {
    filter: blur(0.6px);
  }
  .theater .line.on {
    transform: scale(1.02);
    filter: none;
  }

  .breath {
    display: flex;
    gap: 7px;
    padding: 14px 0;
  }
  .breath span {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--text);
    transition: opacity 0.3s;
  }
  .theater .breath span {
    width: 12px;
    height: 12px;
  }
  .breath.on span {
    animation: breathe 2.4s var(--ease) infinite;
  }
  .breath.on span:nth-child(2) {
    animation-delay: 0.2s;
  }
  .breath.on span:nth-child(3) {
    animation-delay: 0.4s;
  }
  @keyframes breathe {
    50% {
      transform: scale(1.25);
    }
  }

  .tail {
    height: 55%;
  }

  .none {
    height: 100%;
    display: grid;
    align-content: center;
    gap: 6px;
    font-family: var(--font-display);
    font-size: 18px;
    font-weight: 600;
  }
  .none .sub {
    font-family: var(--font-ui);
    font-size: 13px;
    font-weight: 400;
    color: var(--mute);
    text-wrap: pretty;
  }
  .none.theater {
    font-size: 30px;
  }
</style>
