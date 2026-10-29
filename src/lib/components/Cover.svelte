<script lang="ts">
  import { isTauri, type AnyArt } from "$lib/backend";
  import { catalog } from "$lib/catalog.svelte";
  import { hashHue } from "$lib/data";

  // Real covers come through the art:// protocol from an opaque key (D14).
  // Mock art (dev) and missing covers are drawn from a seed instead, using
  // the same colours the app derives from real art.
  let { art, round = false, radius = 8, size = 544, label = "" }: { art: AnyArt; round?: boolean; radius?: number; size?: number; label?: string } = $props();

  type Drawn = { seed: number; h1: number; h2: number; motif: "orbit" | "stripes" | "grid" | "wave" | "sun" | "blocks" };
  const motifs = ["orbit", "stripes", "grid", "wave", "sun", "blocks"] as const;

  const real = $derived(art && "key" in art && !art.key.startsWith("mock:") && isTauri ? art.key : null);
  let failed = $state(false);
  $effect(() => {
    real;
    failed = false;
  });

  const drawn = $derived.by<Drawn>(() => {
    if (art && "seed" in art) return art;
    if (art && "key" in art && art.key.startsWith("mock:")) {
      const [, seed, motif] = art.key.split(":");
      return { seed: +seed, h1: art.h1 ?? 200, h2: art.h2 ?? 240, motif: (motif as Drawn["motif"]) ?? "orbit" };
    }
    const k = art && "key" in art ? art.key : label || "encore";
    const [h1, h2] = catalog.hue(art, k);
    const seed = hashHue(k) * 7919;
    return { seed, h1, h2, motif: motifs[seed % motifs.length] };
  });

  function rng(seed: number) {
    let s = seed >>> 0;
    return () => {
      s = (s + 0x6d2b79f5) >>> 0;
      let t = s;
      t = Math.imul(t ^ (t >>> 15), t | 1);
      t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  }

  const uid = $props.id();
  const shapes = $derived.by(() => {
    const art = drawn;
    const r = rng(art.seed);
    const c1 = `hsl(${art.h1} 62% 58%)`;
    const c2 = `hsl(${art.h2} 55% 30%)`;
    const ink = `hsl(${art.h2} 40% 12%)`;
    const hi = `hsl(${art.h1} 90% 86%)`;
    const out: string[] = [];
    switch (art.motif) {
      case "orbit": {
        const cx = 30 + r() * 40, cy = 30 + r() * 40;
        for (let i = 6; i > 0; i--) out.push(`<circle cx="${cx}" cy="${cy}" r="${i * 11}" fill="none" stroke="${hi}" stroke-opacity="${0.12 + i * 0.03}" stroke-width="${0.6 + r() * 1.4}"/>`);
        out.push(`<circle cx="${cx}" cy="${cy}" r="${7 + r() * 6}" fill="${hi}"/>`);
        out.push(`<circle cx="${cx + 28}" cy="${cy - 18}" r="2.4" fill="${hi}"/>`);
        break;
      }
      case "stripes": {
        for (let i = 0; i < 9; i++) out.push(`<rect x="-20" y="${i * 14 - 10}" width="140" height="${3 + r() * 5}" fill="${i % 2 ? hi : ink}" fill-opacity="${0.25 + r() * 0.5}" transform="rotate(-18 50 50)"/>`);
        break;
      }
      case "grid": {
        for (let x = 0; x < 6; x++)
          for (let y = 0; y < 6; y++)
            if (r() > 0.45) out.push(`<rect x="${8 + x * 14}" y="${8 + y * 14}" width="10" height="10" rx="${r() > 0.5 ? 5 : 1.5}" fill="${r() > 0.7 ? hi : ink}" fill-opacity="${0.35 + r() * 0.6}"/>`);
        break;
      }
      case "wave": {
        for (let i = 0; i < 7; i++) {
          const y = 30 + i * 9, a = 4 + r() * 8;
          out.push(`<path d="M-5 ${y} C 20 ${y - a}, 35 ${y + a}, 55 ${y} S 90 ${y - a}, 105 ${y}" fill="none" stroke="${i % 3 ? hi : ink}" stroke-opacity="${0.3 + i * 0.08}" stroke-width="${1 + r() * 2.5}"/>`);
        }
        break;
      }
      case "sun": {
        const cy = 52 + r() * 10;
        out.push(`<circle cx="50" cy="${cy}" r="${22 + r() * 8}" fill="${hi}" fill-opacity=".9"/>`);
        for (let i = 0; i < 5; i++) out.push(`<rect x="0" y="${cy + 4 + i * 6.5}" width="100" height="${1.6 + i * 0.8}" fill="${c2}"/>`);
        out.push(`<rect x="0" y="${cy + 34}" width="100" height="60" fill="${ink}" fill-opacity=".85"/>`);
        break;
      }
      case "blocks": {
        for (let i = 0; i < 5; i++) {
          const w = 20 + r() * 50, h = 10 + r() * 40;
          out.push(`<rect x="${r() * (100 - w)}" y="${r() * (100 - h)}" width="${w}" height="${h}" fill="${[hi, ink, c1][i % 3]}" fill-opacity="${0.45 + r() * 0.5}" style="mix-blend-mode:${i % 2 ? "screen" : "multiply"}"/>`);
        }
        break;
      }
    }
    return { c1, c2, body: out.join("") };
  });
</script>

{#if real && !failed}
  <img
    class="cover"
    class:round
    style:border-radius={round ? "50%" : `${radius}px`}
    src={`art://localhost/${real}?s=${size}`}
    alt=""
    loading="lazy"
    decoding="async"
    draggable="false"
    onerror={() => (failed = true)}
  />
{:else}
<svg
  class="cover"
  class:round
  style:border-radius={round ? "50%" : `${radius}px`}
  viewBox="0 0 100 100"
  preserveAspectRatio="xMidYMid slice"
  aria-hidden="true"
>
  <defs>
    <linearGradient id="g{uid}" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color={shapes.c1} />
      <stop offset="1" stop-color={shapes.c2} />
    </linearGradient>
  </defs>
  <rect width="100" height="100" fill="url(#g{uid})" />
  {@html shapes.body}
</svg>
{/if}

<style>
  img.cover {
    object-fit: cover;
    background: var(--hover);
  }
  .cover {
    display: block;
    width: 100%;
    height: auto;
    aspect-ratio: 1;
    box-shadow: inset 0 0 0 1px rgb(255 255 255 / 0.06);
  }
</style>
