// Encore landing page. No framework, no dependencies.
// Songs, artists and lyrics are invented, the same demo data the app uses in development.
(() => {
  "use strict";
  const $ = (s, el = document) => el.querySelector(s);
  const $$ = (s, el = document) => [...el.querySelectorAll(s)];
  const root = document.documentElement;
  const reduced = matchMedia("(prefers-reduced-motion: reduce)").matches;
  const L = (rows) => rows.map(([t, text]) => ({ t, text }));
  const fmt = (s) => { s = Math.max(0, Math.floor(s)); return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`; };
  const clamp = (v, a, b) => Math.min(Math.max(v, a), b);

  // ------------------------------------------------------------- data
  const SONGS = [
    { id: "daybreak", title: "Daybreak Signal", artist: "Lumen Drive", dur: 218, art: { seed: 101, h1: 205, h2: 250, motif: "orbit" }, start: 37,
      lyrics: L([[14, "Static on the dashboard, half past four"], [18.5, "Your voice is a rumour I can't ignore"], [23, "Streetlights keep counting what we left behind"], [27.5, "Every exit sign is a question mark"], [32, "Hold on, hold on"], [34.5, "The night's running out of road"], [39, "Turn the dial till the dark gives in"], [43.5, "There's a daybreak signal coming through"], [48, "Faint as a heartbeat, blue on blue"], [52.5, "If you can hear it, say my name"], [57, "We won't be strangers when the morning comes"], [61.5, "Daybreak signal, coming through"], [76, "Paper cups and a map we never read"], [80.5, "Radio ghosts singing what you said"], [85, "Every mile's a promise we forgot to keep"], [89.5, "Still I'm wide awake while the cities sleep"], [94, "Hold on, hold on"], [96.5, "The night's running out of road"], [101, "Turn the dial till the dark gives in"], [105.5, "There's a daybreak signal coming through"], [110, "Faint as a heartbeat, blue on blue"], [114.5, "If you can hear it, say my name"], [119, "We won't be strangers when the morning comes"], [123.5, "Daybreak signal, coming through"], [142, "And if the static swallows every word"], [147, "I'll draw the sunrise from the sounds I heard"], [152, "Ooh, coming through"], [156, "Ooh, coming through"], [160, "There's a daybreak signal coming through"], [164.5, "Faint as a heartbeat, blue on blue"], [169, "If you can hear it, say my name"], [173.5, "We won't be strangers when the morning comes"], [178, "Daybreak signal, coming through"], [186, "Blue on blue"], [192, "Coming through"]]) },
    { id: "yakou", title: "夜行列車", artist: "茜色ノート", dur: 242, art: { seed: 102, h1: 18, h2: 340, motif: "sun" }, start: 30,
      lyrics: L([[12, "窓の外 ながれる街あかり"], [17, "眠れないまま 君をおもう"], [22, "時刻表には ない駅で"], [27, "ふたり 降りられたらいいのに"], [32, "夜行列車 星をこえて"], [37, "まだ知らない 朝へ はしる"], [42, "ことばより 確かなものを"], [47, "この胸に 抱いて ゆこう"], [64, "古いイヤホン 片方ずつ"], [69, "同じメロディ 違う夢"], [74, "トンネルぬけたら 言えるかな"], [79, "さよならじゃなく またねって"], [84, "夜行列車 星をこえて"], [89, "まだ知らない 朝へ はしる"], [94, "ことばより 確かなものを"], [99, "この胸に 抱いて ゆこう"], [118, "ゆれる灯り"], [123, "とおい汽笛"], [134, "夜行列車 星をこえて"], [139, "まだ知らない 朝へ はしる"], [144, "ことばより 確かなものを"], [149, "この胸に 抱いて ゆこう"]]) },
    { id: "noon", title: "Noon Is a Rumour", artist: "Mirei", dur: 199, art: { seed: 104, h1: 305, h2: 20, motif: "orbit" }, start: 25,
      lyrics: L([[9, "Curtains drawn at a quarter to three"], [13.5, "Somebody swears there's a sun out there"], [18, "I heard it once on the radio"], [22.5, "Nobody I know has been"], [27, "Noon is a rumour"], [30, "Noon is a rumour"], [33, "Say it again and I might come out"], [38, "Noon is a rumour"], [41, "Pink static on the ceiling"], [57, "Mirror on the wall says it's half past eight"], [61.5, "Coffee's gone cold but the night can wait"], [66, "You call it morning, I call it late"], [70.5, "Pull the blinds, pull the blinds"], [75, "Noon is a rumour"], [78, "Noon is a rumour"], [81, "Say it again and I might come out"], [86, "Noon is a rumour"], [89, "Pink static on the ceiling"], [112, "Maybe I'll see it"], [116, "Maybe tomorrow"], [120, "Noon is a rumour"], [123, "Noon is a rumour"], [126, "Say it again and I might come out"], [131, "Noon is a rumour"], [134, "Pink static on the ceiling"], [150, "Pull the blinds, pull the blinds"]]) },
    { id: "glass", title: "Glass Weather", artist: "Pale Harbor", dur: 235, art: { seed: 103, h1: 190, h2: 215, motif: "wave" }, start: 28,
      lyrics: L([[11, "Rain on the window writing out your name"], [16, "I keep the kettle on in case you came"], [21, "The forecast said there's nothing left to fear"], [26, "But I can see the sky from here"], [31, "It's glass weather"], [34, "Everything shows"], [37, "Every crack in the light"], [40, "Everybody knows"], [58, "Your coat's still hanging where you let it fall"], [63, "I hear the harbour breathing through the wall"], [68, "It's glass weather"], [71, "Everything shows"], [74, "Every crack in the light"], [77, "Everybody knows"], [104, "It's glass weather"], [107, "Everything shows"], [110, "Every crack in the light"], [113, "Everybody knows"]]) },
  ];
  // Covers for the colour picker. Songs without lyrics get the plain bar, as in the app.
  const COVERS = [
    SONGS[0], SONGS[1], SONGS[2],
    { id: "arcade", title: "Neon Coast", artist: "Velvet Arcade", dur: 233, art: { seed: 106, h1: 335, h2: 265, motif: "blocks" } },
    { id: "sky", title: "Big Sky Theory", artist: "Sora Kanata", dur: 223, art: { seed: 107, h1: 158, h2: 200, motif: "stripes" } },
    { id: "night", title: "3:12 AM, Fourth Floor", artist: "Night Office", dur: 154, art: { seed: 105, h1: 42, h2: 12, motif: "grid" } },
  ];

  // The app's own segment rules (src/lib/data.ts): one per line, tiered by repeats.
  function segments(song) {
    const lines = song.lyrics;
    if (!lines) return null;
    const counts = new Map();
    for (const l of lines) counts.set(l.text, (counts.get(l.text) || 0) + 1);
    return lines.map((l, i) => {
      const next = lines[i + 1] ? lines[i + 1].t : song.dur;
      const est = Math.min(Math.max(l.text.length * 0.11 + 1.6, 2.4), 4.6);
      const end = Math.max(Math.min(next - 0.35, l.t + est), l.t + 0.5);
      const c = counts.get(l.text);
      return { i, start: l.t, end, next, text: l.text, count: c, tier: c >= 3 ? "chorus" : c === 2 ? "hook" : "verse" };
    });
  }
  for (const s of new Set([...SONGS, ...COVERS])) s.segs = segments(s);

  function lineAt(segs, t) {
    if (!segs) return null;
    let f = null;
    for (const s of segs) if (s.start <= t) f = s;
    if (f && t > f.end + 1.5 && f.next - f.end > 6) return null;
    return f;
  }

  // ------------------------------------------------------------- covers
  // Drawn from a seed like the app's mock artwork (Cover.svelte).
  function rng(seed) {
    let s = seed >>> 0;
    return () => {
      s = (s + 0x6d2b79f5) >>> 0;
      let t = s;
      t = Math.imul(t ^ (t >>> 15), t | 1);
      t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  }
  let uid = 0;
  function coverSVG(a) {
    const r = rng(a.seed), id = `cg${++uid}`;
    const c1 = `hsl(${a.h1} 62% 58%)`, c2 = `hsl(${a.h2} 55% 30%)`, ink = `hsl(${a.h2} 40% 12%)`, hi = `hsl(${a.h1} 90% 86%)`;
    const o = [];
    if (a.motif === "orbit") {
      const cx = 30 + r() * 40, cy = 30 + r() * 40;
      for (let i = 6; i > 0; i--) o.push(`<circle cx="${cx}" cy="${cy}" r="${i * 11}" fill="none" stroke="${hi}" stroke-opacity="${0.12 + i * 0.03}" stroke-width="${0.6 + r() * 1.4}"/>`);
      o.push(`<circle cx="${cx}" cy="${cy}" r="${7 + r() * 6}" fill="${hi}"/><circle cx="${cx + 28}" cy="${cy - 18}" r="2.4" fill="${hi}"/>`);
    } else if (a.motif === "stripes") {
      for (let i = 0; i < 9; i++) o.push(`<rect x="-20" y="${i * 14 - 10}" width="140" height="${3 + r() * 5}" fill="${i % 2 ? hi : ink}" fill-opacity="${0.25 + r() * 0.5}" transform="rotate(-18 50 50)"/>`);
    } else if (a.motif === "grid") {
      for (let x = 0; x < 6; x++) for (let y = 0; y < 6; y++) if (r() > 0.45) o.push(`<rect x="${8 + x * 14}" y="${8 + y * 14}" width="10" height="10" rx="${r() > 0.5 ? 5 : 1.5}" fill="${r() > 0.7 ? hi : ink}" fill-opacity="${0.35 + r() * 0.6}"/>`);
    } else if (a.motif === "wave") {
      for (let i = 0; i < 7; i++) { const y = 30 + i * 9, am = 4 + r() * 8; o.push(`<path d="M-5 ${y} C 20 ${y - am}, 35 ${y + am}, 55 ${y} S 90 ${y - am}, 105 ${y}" fill="none" stroke="${i % 3 ? hi : ink}" stroke-opacity="${0.3 + i * 0.08}" stroke-width="${1 + r() * 2.5}"/>`); }
    } else if (a.motif === "sun") {
      const cy = 52 + r() * 10;
      o.push(`<circle cx="50" cy="${cy}" r="${22 + r() * 8}" fill="${hi}" fill-opacity=".9"/>`);
      for (let i = 0; i < 5; i++) o.push(`<rect x="0" y="${cy + 4 + i * 6.5}" width="100" height="${1.6 + i * 0.8}" fill="${c2}"/>`);
      o.push(`<rect x="0" y="${cy + 34}" width="100" height="60" fill="${ink}" fill-opacity=".85"/>`);
    } else if (a.motif === "blocks") {
      for (let i = 0; i < 5; i++) { const w = 20 + r() * 50, h = 10 + r() * 40; o.push(`<rect x="${r() * (100 - w)}" y="${r() * (100 - h)}" width="${w}" height="${h}" fill="${[hi, ink, c1][i % 3]}" fill-opacity="${0.45 + r() * 0.5}" style="mix-blend-mode:${i % 2 ? "screen" : "multiply"}"/>`); }
    }
    return `<svg viewBox="0 0 100 100" preserveAspectRatio="xMidYMid slice" aria-hidden="true" focusable="false"><defs><linearGradient id="${id}" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="${c1}"/><stop offset="1" stop-color="${c2}"/></linearGradient></defs><rect width="100" height="100" fill="url(#${id})"/>${o.join("")}</svg>`;
  }

  // ------------------------------------------------------------- song map
  function SongMap(el, { interactive = false, onseek, label = "Seek" } = {}) {
    let song = null, segEls = [], played, head, tip, drag = null, pos = 0;
    el.innerHTML = "";
    const rail = document.createElement("div"); rail.className = "rail"; el.append(rail);
    if (interactive) {
      el.setAttribute("role", "slider"); el.tabIndex = 0; el.setAttribute("aria-label", label); el.setAttribute("aria-valuemin", "0");
      tip = document.createElement("div"); tip.className = "tip"; tip.innerHTML = `<span class="mono"></span><span class="tl"></span>`;
    }
    const tAt = (x) => { const r = el.getBoundingClientRect(); return clamp((x - r.left) / r.width, 0, 1) * (song ? song.dur : 1); };

    function set(s) {
      song = s;
      $$(".seg, .played, .head, .gapmark", el).forEach((n) => n.remove());
      segEls = [];
      const pct = (t) => `${(t / s.dur) * 100}%`;
      if (s.segs) {
        for (const g of s.segs) {
          const d = document.createElement("div");
          d.className = `seg ${g.tier}`; d.style.left = pct(g.start); d.style.width = pct(g.end - g.start);
          d.append(document.createElement("i"));
          el.append(d); segEls.push(d);
        }
      } else {
        const d = document.createElement("div"); d.className = "seg plain"; d.style.left = "0"; d.style.width = "100%"; d.style.height = "4px";
        d.append(document.createElement("i")); el.append(d); segEls.push(d);
      }
      played = document.createElement("div"); played.className = "played"; el.append(played);
      head = document.createElement("div"); head.className = "head"; el.append(head);
      if (tip) el.append(tip);
      if (interactive) el.setAttribute("aria-valuemax", String(Math.round(s.dur)));
      setPos(pos);
    }
    function setPos(t) {
      pos = drag ?? t;
      if (!song) return;
      const p = pos / song.dur;
      if (song.segs) song.segs.forEach((g, i) => { segEls[i].firstChild.style.transform = `scaleX(${clamp((pos - g.start) / (g.end - g.start), 0, 1)})`; });
      else segEls[0].firstChild.style.transform = `scaleX(${p})`;
      played.style.width = `${p * 100}%`;
      head.style.left = `${p * 100}%`;
      if (interactive) {
        el.setAttribute("aria-valuenow", String(Math.round(pos)));
        const ln = lineAt(song.segs, pos);
        el.setAttribute("aria-valuetext", `${fmt(pos)} of ${fmt(song.dur)}${ln ? `, “${ln.text}”` : ""}`);
      }
    }
    function lit(pred) { if (song && song.segs) song.segs.forEach((g, i) => segEls[i].classList.toggle("lit", !!pred && pred(g))); }

    if (interactive) {
      const showTip = (x, t) => {
        const r = el.getBoundingClientRect(), half = Math.min(150, r.width / 2);
        const ln = lineAt(song.segs, t);
        tip.style.left = `${clamp(x - r.left, half, r.width - half)}px`;
        tip.firstChild.textContent = fmt(t);
        tip.lastChild.textContent = song.segs ? (ln ? ln.text : "Instrumental") : "";
        tip.classList.add("on");
        lit(ln ? (g) => g === ln : null);
      };
      el.addEventListener("pointerdown", (e) => { if (e.button) return; el.setPointerCapture(e.pointerId); drag = tAt(e.clientX); setPos(drag); showTip(e.clientX, drag); onseek && onseek(drag, true); });
      el.addEventListener("pointermove", (e) => { const t = tAt(e.clientX); if (e.pointerType === "mouse" || drag !== null) showTip(e.clientX, t); if (drag !== null) { drag = t; setPos(t); onseek && onseek(t, true); } });
      const end = () => { if (drag !== null) { const t = drag; drag = null; onseek && onseek(t, false); } };
      el.addEventListener("pointerup", (e) => { end(); if (e.pointerType !== "mouse") { tip.classList.remove("on"); lit(null); } });
      el.addEventListener("pointercancel", end);
      el.addEventListener("pointerleave", () => { if (drag === null) { tip.classList.remove("on"); lit(null); } });
      el.addEventListener("keydown", (e) => {
        const step = e.shiftKey ? 15 : 5;
        if (e.key === "ArrowLeft" || e.key === "ArrowDown") onseek(pos - step);
        else if (e.key === "ArrowRight" || e.key === "ArrowUp") onseek(pos + step);
        else if (e.key === "Home") onseek(0);
        else if (e.key === "End") onseek(song.dur - 1);
        else return;
        e.preventDefault(); e.stopPropagation();
      });
    }
    return { set, setPos, lit };
  }

  // ------------------------------------------------------------- lyrics column
  function Lyrics(viewport, track, onpick) {
    let song = null, items = [], cur = -2;
    function set(s) {
      song = s; cur = -2; items = []; track.innerHTML = "";
      const segs = s.segs;
      const gap = (after) => { const g = document.createElement("div"); g.className = "ly gap"; g.innerHTML = "<i></i><i></i><i></i>"; g.setAttribute("aria-hidden", "true"); track.append(g); items.push({ el: g, gap: true, after }); };
      if (segs[0].start > 6) gap(-1);
      segs.forEach((g, i) => {
        const b = document.createElement("button");
        b.type = "button"; b.className = "ly"; b.textContent = g.text; b.tabIndex = -1;
        b.addEventListener("click", () => onpick(g.start));
        track.append(b); items.push({ el: b, seg: g });
        if (g.next - g.end > 6 && i < segs.length - 1) gap(i);
      });
    }
    function setPos(t) {
      if (!song) return;
      const segs = song.segs, ln = lineAt(segs, t);
      let idx;
      if (ln) idx = items.findIndex((it) => it.seg === ln);
      else { let last = -1; for (const g of segs) if (g.start <= t) last = g.i; idx = items.findIndex((it) => it.gap && it.after === last); if (idx < 0) idx = 0; }
      if (idx !== cur) {
        items.forEach((it, i) => { it.el.classList.toggle("on", i === idx); it.el.classList.toggle("past", i < idx); });
        cur = idx;
        const el = items[idx].el;
        track.style.setProperty("--y", `${-(el.offsetTop - viewport.clientHeight * 0.34)}px`);
      }
      const it = items[idx];
      if (it && it.seg) it.el.style.setProperty("--p", `${clamp((t - it.seg.start) / (it.seg.end - it.seg.start), 0, 1) * 100}%`);
    }
    function relayout() { const c = cur; cur = -2; return c; }
    return { set, setPos, relayout };
  }

  // ------------------------------------------------------------- player
  const player = { song: SONGS[0], pos: SONGS[0].start, playing: !reduced, scrubbing: false, liked: new Set() };
  const heroMap = SongMap($("#hero-map"), { interactive: true, label: "Seek the preview", onseek: (t, live) => seek(t, live) });
  const heroLyr = Lyrics($("#lyrics"), $("#lyr-track"), (t) => seek(t));
  const thMap = SongMap($("#th-map"), { interactive: true, label: "Seek", onseek: (t, live) => seek(t, live) });
  const thLyr = Lyrics($("#th-lyrics"), $("#th-track"), (t) => seek(t));
  const deckMap = SongMap($("#deck-map"));
  const nowMap = SongMap($("#now-map"));
  let deckSong = SONGS[0];

  function setHue(a) { root.style.setProperty("--h", a.h1); root.style.setProperty("--h2", a.h2); updateTokens(a.h1); }
  function updateTokens(h) {
    const light = root.dataset.theme === "light";
    $("#tk-accent").textContent = light ? `hsl(${h} 65% 38%)` : `hsl(${h} 78% 74%)`;
    $("#tk-soft").textContent = light ? `hsl(${h} 70% 45% / .12)` : `hsl(${h} 70% 70% / .16)`;
    $("#tk-tint").textContent = light ? `hsl(${h} 80% 70% / .34)` : `hsl(${h} 60% 45% / .32)`;
  }

  function load(song, at) {
    player.song = song;
    player.pos = at ?? 0;
    const svg = coverSVG(song.art);
    $("#cover").innerHTML = svg; $("#th-cover").innerHTML = svg;
    $("#title").textContent = song.title; $("#artist").textContent = song.artist;
    $("#th-title").textContent = song.title; $("#th-artist").textContent = song.artist;
    $("#t-dur").textContent = fmt(song.dur); $("#th-dur").textContent = fmt(song.dur);
    heroMap.set(song); thMap.set(song); heroLyr.set(song); thLyr.set(song);
    $$("#picks .pick").forEach((b) => b.setAttribute("aria-pressed", String(b.dataset.id === song.id)));
    $$("#covers button").forEach((b) => b.setAttribute("aria-checked", String(b.dataset.id === song.id)));
    $$("#covers button").forEach((b) => (b.tabIndex = b.dataset.id === song.id ? 0 : -1));
    setHue(song.art);
    deckSong = song; deckMap.set(song); setDeck(song);
    nowMap.set(song); $("#now-cover").innerHTML = svg; $("#now-title").textContent = song.title;
    $("#like").setAttribute("aria-pressed", String(player.liked.has(song.id)));
    render();
  }
  function setDeck(s) { $("#deck-cover").innerHTML = coverSVG(s.art); $("#deck-title").textContent = s.title; $("#deck-artist").textContent = s.artist; }

  function seek(t, live) {
    player.pos = clamp(t, 0, player.song.dur - 0.05);
    player.scrubbing = !!live;
    hideHint();
    render();
  }
  function render() {
    const p = player.pos;
    heroMap.setPos(p); thMap.setPos(p); heroLyr.setPos(p); thLyr.setPos(p);
    deckMap.setPos(deckSong === player.song ? p : deckSong.dur * 0.38);
    $("#t-pos").textContent = fmt(p); $("#th-pos").textContent = fmt(p);
    nowMap.setPos(p);
    const ln = lineAt(player.song.segs, p), txt = ln ? ln.text : "Instrumental";
    if (nowLine.textContent !== txt) nowLine.textContent = txt;
  }
  const nowLine = $("#now-line");
  function setPlaying(v) {
    player.playing = v;
    $("#now-state").textContent = v ? "Playing" : "Paused";
    const b = $("#play");
    b.setAttribute("aria-pressed", String(v));
    b.setAttribute("aria-label", v ? "Pause preview" : "Play preview");
  }
  function step(dir) {
    const i = SONGS.indexOf(player.song);
    if (dir < 0 && player.pos > 3) return seek(0);
    const n = SONGS[(i + dir + SONGS.length) % SONGS.length];
    load(n, dir > 0 && !reduced ? 0 : n.start);
  }
  function lineStep(dir) {
    const segs = player.song.segs, p = player.pos;
    if (dir > 0) { const n = segs.find((g) => g.start > p + 0.1); seek(n ? n.start : p); }
    else { const c = [...segs].reverse().find((g) => g.start < p - 0.6); const prev = c && [...segs].reverse().find((g) => g.start < c.start); seek(p - (c ? c.start : 0) < 1.2 && prev ? prev.start : c ? c.start : 0); }
  }

  let last = performance.now();
  function tick(now) {
    const dt = Math.min((now - last) / 1000, 0.25); last = now;
    if (player.playing && !player.scrubbing) {
      player.pos += dt;
      if (player.pos >= player.song.dur) step(1);
      render();
    }
    requestAnimationFrame(tick);
  }

  // hero controls
  const picks = $("#picks");
  for (const s of SONGS) {
    const b = document.createElement("button");
    b.type = "button"; b.className = "pick"; b.dataset.id = s.id; b.innerHTML = coverSVG(s.art);
    b.setAttribute("aria-label", `${s.title} by ${s.artist}`); b.setAttribute("aria-pressed", "false");
    b.addEventListener("click", () => { if (s !== player.song) load(s, s.start); setPlaying(true); });
    picks.append(b);
  }
  $("#play").addEventListener("click", () => setPlaying(!player.playing));
  $("#prev").addEventListener("click", () => step(-1));
  $("#next").addEventListener("click", () => step(1));
  $("#shuffle").addEventListener("click", (e) => e.currentTarget.setAttribute("aria-pressed", String(e.currentTarget.getAttribute("aria-pressed") !== "true")));
  $("#like").addEventListener("click", () => toggleLike());
  function toggleLike() { const id = player.song.id; player.liked.has(id) ? player.liked.delete(id) : player.liked.add(id); $("#like").setAttribute("aria-pressed", String(player.liked.has(id))); }
  let hintGone = false;
  function hideHint() { if (hintGone) return; hintGone = true; const h = $("#hint"); h.style.opacity = "0"; }

  // ------------------------------------------------------------- anatomy
  (function anatomy() {
    const song = SONGS[0], fig = $(".anatomy");
    const m = SongMap($("#anat-map"));
    m.set(song); m.setPos(0);
    const host = $("#anat-map");
    // dashed marks under instrumental stretches
    song.segs.forEach((g, i) => {
      const from = i === 0 ? 0 : g.end; const to = i === 0 ? g.start : g.next;
      if (i === 0 && g.start > 6) mark(0, g.start);
      if (g.next - g.end > 6 && i < song.segs.length - 1) mark(g.end, g.next);
      if (i === song.segs.length - 1 && song.dur - g.end > 6) mark(g.end, song.dur);
      void from; void to;
    });
    function mark(a, b) { const d = document.createElement("div"); d.className = "gapmark"; d.style.left = `${(a / song.dur) * 100 + 0.4}%`; d.style.width = `${((b - a) / song.dur) * 100 - 0.8}%`; host.append(d); }
    const sheet = $("#sheet"), seen = new Set();
    for (const g of song.segs) {
      if (seen.has(g.text)) continue; seen.add(g.text);
      const li = document.createElement("li"), b = document.createElement("button");
      b.type = "button"; b.className = g.tier;
      b.innerHTML = `<span class="t">${fmt(g.start)}</span><span></span><span class="x">${g.count > 1 ? "×" + g.count : ""}</span>`;
      b.children[1].textContent = g.text;
      b.setAttribute("aria-label", `${g.text}. Sung ${g.count === 1 ? "once" : g.count === 2 ? "twice" : g.count + " times"}. Play it in the preview.`);
      const on = () => { fig.classList.add("focus"); b.classList.add("lit"); m.lit((x) => x.text === g.text); };
      const off = () => { fig.classList.remove("focus"); b.classList.remove("lit"); m.lit(null); };
      b.addEventListener("pointerenter", on); b.addEventListener("pointerleave", off);
      b.addEventListener("focus", on); b.addEventListener("blur", off);
      b.addEventListener("click", () => { if (player.song !== song) load(song); seek(g.start); setPlaying(true); $("#top").scrollIntoView({ behavior: reduced ? "auto" : "smooth" }); });
      li.append(b); sheet.append(li);
    }
  })();

  // ------------------------------------------------------------- colour
  (function colour() {
    const host = $("#covers");
    COVERS.forEach((s) => {
      const b = document.createElement("button");
      b.type = "button"; b.dataset.id = s.id; b.setAttribute("role", "radio"); b.setAttribute("aria-checked", "false");
      b.setAttribute("aria-label", `${s.title} by ${s.artist}`);
      b.innerHTML = coverSVG(s.art) + `<span class="cl"></span>`;
      b.lastChild.textContent = s.title;
      b.addEventListener("click", () => pickCover(s));
      host.append(b);
    });
    host.addEventListener("keydown", (e) => {
      const bs = $$("button", host), i = bs.indexOf(document.activeElement);
      const d = { ArrowRight: 1, ArrowDown: 3, ArrowLeft: -1, ArrowUp: -3 }[e.key];
      if (i < 0 || !d) return;
      e.preventDefault();
      const n = bs[(i + d + bs.length) % bs.length]; n.focus(); n.click();
    });
    function pickCover(s) {
      if (s.lyrics) { if (s !== player.song) load(s, s.start); return; }
      $$("#covers button").forEach((b) => { b.setAttribute("aria-checked", String(b.dataset.id === s.id)); b.tabIndex = b.dataset.id === s.id ? 0 : -1; });
      setHue(s.art); deckSong = s; deckMap.set(s); setDeck(s); render();
    }
  })();

  // ------------------------------------------------------------- screens
  const shot = { view: "home", os: ["windows", "linux"].includes(root.dataset.os) ? root.dataset.os : "macos" };
  const VIEW_ALT = { home: "home, with lyrics and the queue in the side panel", lyrics: "full-screen lyrics", palette: "the search and command palette", settings: "settings" };
  const OS_NAME = { macos: "macOS", windows: "Windows", linux: "Linux" };
  function shotSrc(w, v = shot.view, os = shot.os) { return `assets/shots/${os}-${v}-${root.dataset.theme}-${w}.webp`; }
  function showShot() {
    const img = $("#shot-img"), win = $("#shot");
    const src = shotSrc(1100), set = `${shotSrc(1100)} 1100w, ${shotSrc(2000)} 2000w`;
    win.className = `window os-${shot.os}`;
    if (img.getAttribute("src") === src) return;
    const alt = `Encore on ${OS_NAME[shot.os]}, ${root.dataset.theme} theme: ${VIEW_ALT[shot.view]}`;
    // Until the viewer is near the viewport, just retarget the lazy image.
    if (!shotSeen) { img.srcset = set; img.src = src; img.alt = alt; return; }
    const pre = new Image(); pre.sizes = img.sizes; pre.srcset = set; pre.src = src;
    img.classList.add("loading");
    const swap = () => { img.srcset = set; img.src = src; img.alt = alt; img.classList.remove("loading"); };
    pre.decode ? pre.decode().then(swap, swap) : (pre.onload = swap);
  }
  let shotSeen = !("IntersectionObserver" in window);
  if (!shotSeen) { const io = new IntersectionObserver((es) => { if (es.some((e) => e.isIntersecting)) { shotSeen = true; io.disconnect(); } }, { rootMargin: "800px 0px" }); io.observe($("#shot")); }
  const tabs = $$("#views [role=tab]");
  tabs.forEach((t) => {
    t.addEventListener("click", () => selectTab(t));
    t.addEventListener("pointerenter", () => { if (!shotSeen) return; const i = new Image(); i.src = shotSrc(innerWidth > 1100 ? 2000 : 1100, t.dataset.view); });
  });
  function selectTab(t, focus) {
    tabs.forEach((x) => { const on = x === t; x.setAttribute("aria-selected", String(on)); x.tabIndex = on ? 0 : -1; });
    $("#shot").setAttribute("aria-labelledby", t.id);
    shot.view = t.dataset.view; showShot();
    if (focus) t.focus();
  }
  $("#views").addEventListener("keydown", (e) => {
    const i = tabs.indexOf(document.activeElement); if (i < 0) return;
    const d = { ArrowDown: 1, ArrowRight: 1, ArrowUp: -1, ArrowLeft: -1 }[e.key];
    if (e.key === "Home") { e.preventDefault(); return selectTab(tabs[0], true); }
    if (e.key === "End") { e.preventDefault(); return selectTab(tabs[tabs.length - 1], true); }
    if (!d) return; e.preventDefault(); selectTab(tabs[(i + d + tabs.length) % tabs.length], true);
  });
  const osBtns = $$("#os-ctl button");
  function selectOS(os, focus) {
    shot.os = os;
    osBtns.forEach((b) => { const on = b.dataset.os === os; b.setAttribute("aria-checked", String(on)); b.tabIndex = on ? 0 : -1; if (on && focus) b.focus(); });
    showShot();
  }
  osBtns.forEach((b) => b.addEventListener("click", () => selectOS(b.dataset.os)));
  $("#os-ctl").addEventListener("keydown", (e) => {
    const i = osBtns.indexOf(document.activeElement); if (i < 0) return;
    const d = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 }[e.key]; if (!d) return;
    e.preventDefault(); selectOS(osBtns[(i + d + 3) % 3].dataset.os, true);
  });

  // ------------------------------------------------------------- theme
  const themeBtn = $("#theme");
  function applyTheme(t, save) {
    root.dataset.theme = t;
    themeBtn.setAttribute("aria-label", t === "dark" ? "Switch to light theme" : "Switch to dark theme");
    if (save) try { localStorage.setItem("encore-theme", t); } catch (e) {}
    $$('meta[name="theme-color"]').forEach((m) => (m.content = t === "dark" ? "#0e1014" : "#f2f3f5"));
    updateTokens(+getComputedStyle(root).getPropertyValue("--h") || player.song.art.h1);
    showShot();
  }
  themeBtn.addEventListener("click", () => applyTheme(root.dataset.theme === "dark" ? "light" : "dark", true));

  // ------------------------------------------------------------- OS + download
  const isMobile = /android|iphone|ipad|ipod/i.test(navigator.userAgent) || (navigator.maxTouchPoints > 1 && /mac/i.test(navigator.platform) && !matchMedia("(pointer: fine)").matches);
  const isMac = root.dataset.os === "macos";
  (function downloads() {
    const os = isMobile ? null : root.dataset.os;
    root.dataset.dl = os || "none";
    $$(".osname").forEach((n) => (n.textContent = os ? OS_NAME[os] : "your computer"));
    $$(".dl-main").forEach((a) => {
      const svg = $("svg", a);
      if (os) $("use", svg).setAttribute("href", `#i-${os}`); else svg.remove();
      if (!os) $("span", a).innerHTML = "Download Encore";
    });
    $$("#platforms li").forEach((li) => li.classList.toggle("cur", li.dataset.os === os));
    if (!isMac) {
      $$(".modk").forEach((k) => (k.textContent = "Ctrl K"));
      $$(".shiftk").forEach((k) => (k.textContent = "Shift"));
      $(".modk-arrow").textContent = "Ctrl ←"; $(".modk-arrow2").textContent = "Ctrl →";
    }
    $$(".copy").forEach((b) => b.addEventListener("click", async () => {
      const text = b.previousElementSibling.textContent;
      try { await navigator.clipboard.writeText(text); b.textContent = "Copied"; b.classList.add("done"); }
      catch (e) { const r = document.createRange(); r.selectNodeContents(b.previousElementSibling); getSelection().removeAllRanges(); getSelection().addRange(r); b.textContent = "Selected"; }
      setTimeout(() => { b.textContent = "Copy"; b.classList.remove("done"); }, 1600);
    }));
  })();

  // ------------------------------------------------------------- theater (F)
  const theater = $("#theater");
  let lastFocus = null;
  function openTheater() {
    if (!theater.hidden) return;
    lastFocus = document.activeElement; theater.hidden = false; document.body.style.overflow = "hidden";
    thLyr.relayout(); render(); $("#th-close").focus();
  }
  function closeTheater() { if (theater.hidden) return; theater.hidden = true; document.body.style.overflow = ""; heroLyr.relayout(); render(); lastFocus && lastFocus.focus && lastFocus.focus(); }
  $("#th-close").addEventListener("click", closeTheater);
  theater.addEventListener("keydown", (e) => {
    if (e.key !== "Tab") return;
    const f = [$("#th-close"), $("#th-map")];
    const i = f.indexOf(document.activeElement);
    e.preventDefault(); f[(i + (e.shiftKey ? -1 : 1) + f.length) % f.length].focus();
  });

  // ------------------------------------------------------------- palette (⌘K)
  const pal = $("#palette"), q = $("#pal-q"), list = $("#pal-list");
  const G = (d) => `<span class="pi g"><svg viewBox="0 0 20 20">${d}</svg></span>`;
  const ICON = {
    play: G('<path d="M6.5 4v12L16 10z"/>'), next: G('<path d="M5 4.5v11l8-5.5zM15 4.5v11"/>'), prev: G('<path d="M15 4.5v11L7 10zM5 4.5v11"/>'),
    full: G('<path d="M3.5 7.5v-4h4M16.5 7.5v-4h-4M3.5 12.5v4h4M16.5 12.5v4h-4"/>'), theme: G('<circle cx="10" cy="10" r="6.5"/><path d="M10 3.5v13a6.5 6.5 0 0 0 0-13Z" fill="currentColor"/>'),
    heart: G('<path d="M10 16.5s-6.5-3.8-6.5-8.4A3.6 3.6 0 0 1 10 6a3.6 3.6 0 0 1 6.5 2.1c0 4.6-6.5 8.4-6.5 8.4Z"/>'), go: G('<path d="M4 10h11M11 5.5 15.5 10 11 14.5"/>'),
    dl: G('<path d="M10 3.5v9M6 8.5l4 4 4-4M4 16.5h12"/>'), code: G('<path d="m7 6-4 4 4 4M13 6l4 4-4 4"/>'),
  };
  function commands() {
    const t = root.dataset.theme;
    return [
      ...SONGS.map((s) => ({ g: "Songs", label: s.title, sub: `${s.artist} · ${fmt(s.dur)}`, icon: `<span class="pi">${coverSVG(s.art)}</span>`, run: () => { load(s, s.start); setPlaying(true); }, k: "song " + s.artist })),
      { g: "Player", label: player.playing ? "Pause" : "Play", icon: ICON.play, key: "Space", run: () => setPlaying(!player.playing), k: "play pause toggle" },
      { g: "Player", label: "Next song", icon: ICON.next, key: isMac ? "⌘→" : "Ctrl →", run: () => step(1), k: "skip forward" },
      { g: "Player", label: "Previous song", icon: ICON.prev, key: isMac ? "⌘←" : "Ctrl ←", run: () => step(-1), k: "back" },
      { g: "Player", label: "Full-screen lyrics", icon: ICON.full, key: "F", run: openTheater, k: "theater lyrics fullscreen" },
      { g: "Player", label: player.liked.has(player.song.id) ? "Remove like" : "Like this song", icon: ICON.heart, key: "L", run: toggleLike, k: "heart favourite favorite" },
      { g: "Page", label: t === "dark" ? "Switch to light theme" : "Switch to dark theme", icon: ICON.theme, run: () => applyTheme(t === "dark" ? "light" : "dark", true), k: "theme appearance mode" },
      ...[["map", "The song map"], ["screens", "The app"], ["colour", "Colour"], ["keys", "Keys"], ["details", "Details"], ["download", "Download"]].map(([id, label]) => ({ g: "Page", label: `Go to ${label.toLowerCase().replace(/^the /, "the ")}`, icon: ICON.go, run: () => goTo(id), k: "section jump scroll " + id })),
      ...["macos", "windows", "linux"].map((os) => ({ g: "Get Encore", label: `Download for ${OS_NAME[os]}`, icon: ICON.dl, run: () => (location.href = "https://github.com/Nayhap/encore/releases/latest"), k: "install release " + os })),
      { g: "Get Encore", label: "Source on GitHub", icon: ICON.code, run: () => (location.href = "https://github.com/Nayhap/encore"), k: "code repository open source" },
    ];
  }
  function goTo(id) { const el = document.getElementById(id); el.scrollIntoView({ behavior: reduced ? "auto" : "smooth" }); const h = $("h2", el); if (h) { h.tabIndex = -1; h.focus({ preventScroll: true }); } }
  let items = [], sel = 0;
  function score(c, s) {
    if (!s) return 1;
    const hay = (c.label + " " + (c.sub || "") + " " + (c.k || "")).toLowerCase();
    if (c.label.toLowerCase().startsWith(s)) return 3;
    if (hay.includes(s)) return 2;
    let i = 0; for (const ch of hay) if (ch === s[i]) i++;
    return i === s.length ? 0.5 : 0;
  }
  function draw() {
    const s = q.value.trim().toLowerCase();
    items = commands().map((c) => ({ c, sc: score(c, s) })).filter((x) => x.sc > 0);
    if (s) items.sort((a, b) => b.sc - a.sc);
    items = items.map((x) => x.c);
    sel = clamp(sel, 0, Math.max(0, items.length - 1));
    list.innerHTML = "";
    if (!items.length) { list.innerHTML = `<li class="pal-empty" role="presentation">Nothing here matches. The app’s palette searches all of YouTube Music.</li>`; q.removeAttribute("aria-activedescendant"); return; }
    let g = null;
    items.forEach((c, i) => {
      if (!s && c.g !== g) { g = c.g; const h = document.createElement("li"); h.className = "pal-group"; h.setAttribute("role", "presentation"); h.textContent = g; list.append(h); }
      const li = document.createElement("li");
      li.className = "pal-item"; li.id = `pal-${i}`; li.setAttribute("role", "option"); li.setAttribute("aria-selected", String(i === sel));
      li.innerHTML = `${c.icon}<span class="pt"><span></span>${c.sub ? "<small></small>" : ""}</span>${c.key ? `<span class="pk">${c.key}</span>` : ""}`;
      $(".pt span", li).textContent = c.label; if (c.sub) $(".pt small", li).textContent = c.sub;
      li.addEventListener("click", () => runItem(i));
      li.addEventListener("pointermove", () => { if (sel !== i) { sel = i; mark(); } });
      list.append(li);
    });
    mark();
  }
  function mark() { $$(".pal-item", list).forEach((li) => li.setAttribute("aria-selected", String(li.id === `pal-${sel}`))); q.setAttribute("aria-activedescendant", `pal-${sel}`); const el = $(`#pal-${sel}`); el && el.scrollIntoView({ block: "nearest" }); }
  function runItem(i) { const c = items[i]; if (!c) return; closePalette(true); c.run(); }
  let palFocus = null;
  function openPalette() { if (!pal.hidden) return q.select(); palFocus = document.activeElement; pal.hidden = false; q.value = ""; sel = 0; draw(); q.focus(); }
  function closePalette(running) { if (pal.hidden) return; pal.hidden = true; if (!running && palFocus && palFocus.focus) palFocus.focus(); }
  q.addEventListener("input", () => { sel = 0; draw(); });
  q.addEventListener("keydown", (e) => {
    if (e.key === "ArrowDown") { e.preventDefault(); sel = (sel + 1) % items.length; mark(); }
    else if (e.key === "ArrowUp") { e.preventDefault(); sel = (sel - 1 + items.length) % items.length; mark(); }
    else if (e.key === "Enter") { e.preventDefault(); runItem(sel); }
    else if (e.key === "Tab") e.preventDefault();
  });
  pal.addEventListener("pointerdown", (e) => { if (e.target === pal) closePalette(); });
  $$("[data-palette]").forEach((b) => b.addEventListener("click", openPalette));

  // ------------------------------------------------------------- keyboard
  const keyRows = Object.fromEntries($$("#keylist li").map((li) => [li.dataset.k, li]));
  function flash(k) { const li = keyRows[k]; if (!li) return; li.classList.add("hit"); clearTimeout(li._t); li._t = setTimeout(() => li.classList.remove("hit"), 420); }
  document.addEventListener("keydown", (e) => {
    const mod = isMac ? e.metaKey : e.ctrlKey;
    if (mod && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "k") { e.preventDefault(); flash("k"); pal.hidden ? openPalette() : closePalette(); return; }
    if (e.key === "Escape") { if (!pal.hidden) { closePalette(); flash("esc"); } else if (!theater.hidden) { closeTheater(); flash("esc"); } return; }
    if (!pal.hidden) return;
    const tg = e.target, tag = tg.tagName;
    if (tag === "INPUT" || tag === "TEXTAREA" || tg.isContentEditable) return;
    if (e.altKey) return;
    const interactive = tg.closest && tg.closest("button, a, [role=tab], [role=radio], [role=slider], summary");
    if (mod && (e.key === "ArrowLeft" || e.key === "ArrowRight")) { e.preventDefault(); flash("song"); step(e.key === "ArrowRight" ? 1 : -1); return; }
    if (e.metaKey || e.ctrlKey) return;
    switch (e.key) {
      case " ": if (interactive && !interactive.matches("[role=slider]")) return; e.preventDefault(); flash("space"); setPlaying(!player.playing); break;
      case "ArrowLeft": case "ArrowRight": if (interactive) return; e.preventDefault(); flash("seek"); seek(player.pos + (e.key === "ArrowRight" ? 1 : -1) * (e.shiftKey ? 15 : 5)); break;
      case "[": case "]": flash("line"); lineStep(e.key === "]" ? 1 : -1); break;
      case "f": case "F": flash("f"); theater.hidden ? openTheater() : closeTheater(); break;
      case "l": case "L": flash("l"); toggleLike(); break;
      case "s": case "S": $("#shuffle").click(); break;
      default: return;
    }
  });
  // The slider handles its own arrows; light up the row for those too.
  $("#hero-map").addEventListener("keydown", (e) => { if (e.key === "ArrowLeft" || e.key === "ArrowRight") flash("seek"); }, true);

  // ------------------------------------------------------------- page map + nav
  const nav = $("#nav"), pm = $(".pagemap"), pmHead = $(".pm-head");
  const pmSegs = $$(".ps", pm).map((a) => ({ a, el: document.getElementById(a.getAttribute("href").slice(1)), from: parseFloat(a.style.getPropertyValue("--a")), to: parseFloat(a.style.getPropertyValue("--b")) }));
  let ticking = false;
  function onScroll() {
    ticking = false;
    nav.classList.toggle("scrolled", scrollY > 8);
    if (getComputedStyle(pm).display === "none") return;
    const probe = scrollY + innerHeight * 0.45;
    const atEnd = scrollY + innerHeight >= document.documentElement.scrollHeight - 4;
    let headX = 0;
    pmSegs.forEach((s, i) => {
      const top = s.el.offsetTop, h = s.el.offsetHeight;
      const f = atEnd && i === pmSegs.length - 1 ? 1 : clamp((probe - top) / h, 0, 1);
      s.a.style.setProperty("--fill", f);
      if (probe >= top) headX = s.from + (s.to - s.from) * f;
      if (f > 0 && f < 1) s.a.setAttribute("aria-current", "true"); else s.a.removeAttribute("aria-current");
    });
    if (probe < pmSegs[0].el.offsetTop) headX = pmSegs[0].from * clamp(probe / pmSegs[0].el.offsetTop, 0, 1);
    pmHead.style.setProperty("--p", `${headX}%`);
  }
  addEventListener("scroll", () => { if (!ticking) { ticking = true; requestAnimationFrame(onScroll); } }, { passive: true });
  addEventListener("resize", () => { heroLyr.relayout(); thLyr.relayout(); render(); onScroll(); });

  // Footer: the whole song, played. That's the encore.
  (function foot() { const m = SongMap($("#foot-map")); m.set(SONGS[0]); m.setPos(SONGS[0].dur); })();

  // Reveal sections as they arrive.
  if (!reduced && "IntersectionObserver" in window) {
    const els = $$(".sec-head, .anatomy, .viewer, .colour-copy, .colour-play, .keys-grid > *, .facts > div, .dl-card");
    els.forEach((el) => el.classList.add("rv"));
    const io = new IntersectionObserver((es) => es.forEach((en) => { if (en.isIntersecting) { en.target.classList.add("in"); io.unobserve(en.target); } }), { rootMargin: "0px 0px -8% 0px" });
    els.forEach((el) => io.observe(el));
  }

  // ------------------------------------------------------------- start
  load(SONGS[0], SONGS[0].start);
  setPlaying(player.playing);
  selectOS(shot.os);
  applyTheme(root.dataset.theme, false);
  onScroll();
  // Re-centre once fonts change line heights.
  document.fonts && document.fonts.ready.then(() => { heroLyr.relayout(); thLyr.relayout(); render(); });
  requestAnimationFrame((n) => { last = n; tick(n); });

  // Test hook for automated checks.
  window.__encore = { seek, setPlaying, load, songs: SONGS, player, openPalette, openTheater };
})();
