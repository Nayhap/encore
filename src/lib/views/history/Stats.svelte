<script lang="ts">
  // History → Stats (D30): what you played in Encore, by month or year,
  // from the local play log. Works signed out and offline; nothing leaves
  // the computer.
  import { call, type ListeningStats, type StatsOverview, type StatsPeriod, type TopCredit, type Track } from "$lib/backend";
  import { catalog } from "$lib/catalog.svelte";
  import { artistLine } from "$lib/data";
  import { Resource } from "$lib/load.svelte";
  import { listening, minutesText, nav, periodName, player, trackMenu, ui } from "$lib/state.svelte";
  import Cover from "$lib/components/Cover.svelte";
  import Status from "$lib/components/Status.svelte";

  const SHORT = 5;
  const today = new Date();
  const thisMonth: StatsPeriod = { year: today.getFullYear(), month: today.getMonth() + 1 };
  const same = (a: StatsPeriod, b: StatsPeriod) => a.year === b.year && a.month === b.month;

  let period = $state<StatsPeriod>(thisMonth);
  let all = $state(false);

  const overview = new Resource<StatsOverview>(() => call((a) => a.features.stats.overview()));
  const res = new Resource<ListeningStats>(() => call((a) => a.features.stats.period($state.snapshot(period))), false);
  $effect(() => {
    period;
    all = false;
    void res.reload();
  });
  // A play was counted or the log cleared.
  let seen = listening.rev;
  $effect(() => {
    const r = listening.rev;
    if (r === seen) return;
    seen = r;
    void overview.reload();
    void res.reload();
  });

  // This month first, then every month with plays, then the years.
  const months = $derived.by(() => {
    const m = overview.data?.months ?? [];
    return m.some((x) => same(x, thisMonth)) ? m : [thisMonth, ...m];
  });
  const years = $derived([...new Set(months.map((m) => m.year))].map((year): StatsPeriod => ({ year, month: null })));
  const s = $derived(res.data && same(res.data.period, period) ? res.data : null);
  const none = $derived(overview.data && !overview.data.months.length);

  const top = <T,>(xs: T[]) => (all ? xs : xs.slice(0, SHORT));
  const plays = (n: number) => (n === 1 ? "1 play" : `${n.toLocaleString()} plays`);

  // The by-hour strip: bars scaled to the busiest hour.
  const hours = $derived((s?.byHour ?? []).map((m) => m ?? 0));
  const peak = $derived(Math.max(0, ...hours));
  const peakHour = $derived(peak > 0 ? hours.indexOf(peak) : null);
  const hourName = (h: number) => (h === 0 ? "midnight" : h === 12 ? "noon" : h < 12 ? `${h} am` : `${h - 12} pm`);
  const since = $derived(overview.data?.since ? new Date(Number(overview.data.since) * 1000).toLocaleDateString(undefined, { day: "numeric", month: "long", year: "numeric" }) : null);

  function playFrom(tracks: Track[], i: number, label: string) {
    const t = tracks[i];
    if (player.current?.id === t.id) return player.toggle();
    player.playTracks(tracks, i, { label, collectionId: null, radioPlaylist: null });
  }
  const songs = $derived(s?.topSongs.map((x) => x.track) ?? []);
  const label = $derived(`Your top songs · ${periodName(period)}`);

  const artistArt = (a: TopCredit) => (a.id ? catalog.artist(a.id)?.art : null) ?? a.art;
</script>

{#if none}
  <div class="empty">
    <b>Your stats start with your next song</b>
    <p>Encore counts what you play on this computer: a song counts once you've heard half of it, or four minutes. Nothing leaves your computer.</p>
  </div>
{:else}
  <div class="periods" role="group" aria-label="Period">
    {#each months as m (m.year * 100 + (m.month ?? 0))}
      <button class="chip" aria-pressed={same(m, period)} onclick={() => (period = m)}>{periodName(m)}</button>
    {/each}
    <span class="sep" aria-hidden="true"></span>
    {#each years as y (y.year)}
      <button class="chip" aria-pressed={same(y, period)} onclick={() => (period = y)}>{y.year}</button>
    {/each}
  </div>

  <Status loading={res.loading && !s} error={res.error} onretry={() => res.reload()} />

  {#if s && !s.plays && !s.minutes}
    <p class="quiet">Nothing played in {periodName(period)}{same(period, thisMonth) ? " yet" : ""}.</p>
  {:else if s}
    <div class="figures" class:busy={res.loading}>
      <div><b class="mono">{minutesText(s.minutes)}</b><span>listened</span></div>
      <div><b class="mono">{s.plays.toLocaleString()}</b><span>{s.plays === 1 ? "play" : "plays"}</span></div>
      <div><b class="mono">{s.songs.toLocaleString()}</b><span>{s.songs === 1 ? "song" : "songs"}</span></div>
      <div class="hours">
        <div class="bars" role="img" aria-label={peakHour != null ? `Listening by hour of day, most around ${hourName(peakHour)}` : "Listening by hour of day"}>
          {#each hours as m, h}
            <i class:peak={h === peakHour} style:--v={peak ? Math.max(m / peak, m > 0 ? 0.08 : 0) : 0} title="{hourName(h)} · {minutesText(m)}"></i>
          {/each}
        </div>
        <span>{peakHour != null ? `Mostly around ${hourName(peakHour)}` : "By hour of day"}</span>
      </div>
    </div>

    <div class="cols">
      <section>
        <h2>Top songs</h2>
        <ol class="rank">
          {#each top(s.topSongs) as x, i (x.track.id)}
            {@const cur = player.current?.id === x.track.id}
            <li>
              <button class="row" class:cur onclick={() => playFrom(songs, i, label)} oncontextmenu={(e) => ui.openMenu(e, trackMenu(x.track.id))} aria-label="Play {x.track.title}">
                <span class="n mono">{i + 1}</span>
                <Cover art={x.track.art} size={80} radius={6} label={x.track.title} />
                <span class="t"><b class="clamp1">{x.track.title}</b><small class="clamp1">{artistLine(x.track)}</small></span>
                <span class="c">{plays(x.plays)}</span>
              </button>
            </li>
          {/each}
        </ol>
      </section>
      <section>
        <h2>Top artists</h2>
        <ol class="rank">
          {#each top(s.topArtists) as a, i (a.id ?? a.name)}
            <li>
              <button class="row" disabled={!a.id} onclick={() => a.id && nav.go({ name: "artist", id: a.id })}>
                <span class="n mono">{i + 1}</span>
                <Cover art={artistArt(a)} size={80} round label={a.name} />
                <span class="t"><b class="clamp1">{a.name}</b><small>{minutesText(a.minutes)}</small></span>
                <span class="c">{plays(a.plays)}</span>
              </button>
            </li>
          {/each}
        </ol>
      </section>
    </div>
    {#if s.topSongs.length > SHORT || s.topArtists.length > SHORT}
      <button class="pill more" onclick={() => (all = !all)}>{all ? "Show fewer" : "Show top 10"}</button>
    {/if}

    {#if s.topAlbums.length}
      <section>
        <h2>Top albums</h2>
        <div class="albums">
          {#each s.topAlbums.slice(0, 6) as al (al.id ?? al.name)}
            <button class="album" disabled={!al.id} onclick={() => al.id && nav.go({ name: "collection", id: al.id })}>
              <Cover art={al.art} size={240} radius={8} label={al.name} />
              <b class="clamp1">{al.name}</b>
              <small class="clamp1">{al.detail} · {plays(al.plays)}</small>
            </button>
          {/each}
        </div>
      </section>
    {/if}

    {#if s.newSongs}
      <section class="new">
        <h2>New to you</h2>
        <p>{s.newSongs === 1 ? "1 song" : `${s.newSongs.toLocaleString()} songs`} you played for the first time{period.month == null ? ` in ${period.year}` : same(period, thisMonth) ? " this month" : ` in ${periodName(period)}`}.</p>
        <div class="fresh">
          {#each s.newTop as t, i (t.id)}
            <button onclick={() => playFrom(s.newTop, i, `New to you · ${periodName(period)}`)} oncontextmenu={(e) => ui.openMenu(e, trackMenu(t.id))} title="{t.title} · {artistLine(t)}" aria-label="Play {t.title}">
              <Cover art={t.art} size={120} radius={6} label={t.title} />
            </button>
          {/each}
        </div>
      </section>
    {/if}
  {/if}

  {#if since}
    <p class="foot">Counted on this computer since {since}. A song counts once you've heard half of it, or four minutes.</p>
  {/if}
{/if}

<style>
  .periods {
    display: flex;
    gap: 6px;
    margin-top: 18px;
    overflow-x: auto;
    scrollbar-width: none;
    padding-bottom: 2px;
  }
  .periods .chip {
    flex: none;
  }
  .sep {
    flex: none;
    width: 1px;
    margin: 6px 4px;
    background: var(--line);
  }
  .figures {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: 14px 36px;
    margin: 26px 0 8px 4px;
    transition: opacity 0.2s;
  }
  .figures.busy {
    opacity: 0.6;
  }
  .figures > div {
    display: grid;
    gap: 2px;
  }
  .figures b {
    font-family: var(--font-display);
    font-size: 30px;
    font-weight: 700;
    letter-spacing: -0.02em;
    line-height: 1.1;
  }
  .figures span {
    font-size: 12.5px;
    color: var(--mute);
  }
  .hours {
    margin-left: auto;
  }
  .bars {
    display: flex;
    align-items: flex-end;
    gap: 2px;
    height: 34px;
  }
  .bars i {
    width: 6px;
    height: calc(3px + var(--v) * 31px);
    border-radius: 2px;
    background: color-mix(in srgb, var(--accent) calc(25% + var(--v) * 60%), var(--hover));
  }
  .bars i.peak {
    background: var(--accent);
  }
  .cols {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
    gap: 8px 36px;
  }
  section {
    margin-top: 22px;
  }
  h2 {
    font-family: var(--font-display);
    font-size: 19px;
    font-weight: 650;
    letter-spacing: -0.01em;
    margin: 0 0 8px 4px;
  }
  .rank {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .row {
    display: grid;
    grid-template-columns: 22px 40px 1fr auto;
    align-items: center;
    gap: 12px;
    width: 100%;
    padding: 6px 10px 6px 4px;
    border-radius: 10px;
    text-align: left;
  }
  .row :global(.cover) {
    width: 40px;
    height: 40px;
  }
  .row:not(:disabled):hover {
    background: var(--hover);
  }
  .row.cur b {
    color: var(--accent);
  }
  .n {
    text-align: right;
    color: var(--dim);
    font-size: 13px;
  }
  .t {
    display: grid;
    min-width: 0;
  }
  .t b {
    font-weight: 600;
    font-size: 14px;
  }
  .t small,
  .c {
    font-size: 12.5px;
    color: var(--mute);
  }
  .c {
    font-variant-numeric: tabular-nums;
  }
  .clamp1 {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .more {
    margin: 6px 0 0 4px;
  }
  .albums {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(132px, 1fr));
    gap: 16px;
  }
  .album {
    display: grid;
    gap: 2px;
    min-width: 0;
    padding: 4px;
    border-radius: 10px;
    text-align: left;
  }
  .album:not(:disabled):hover {
    background: var(--hover);
  }
  .album :global(.cover) {
    width: 100%;
    aspect-ratio: 1;
    height: auto;
    margin-bottom: 6px;
  }
  .album b {
    font-size: 13.5px;
    font-weight: 600;
  }
  .album small {
    font-size: 12px;
    color: var(--mute);
  }
  .new p {
    margin: -4px 0 10px 4px;
    font-size: 13px;
    color: var(--mute);
  }
  .fresh {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-left: 4px;
  }
  .fresh button {
    width: 56px;
    height: 56px;
    border-radius: 6px;
    transition: transform 0.15s var(--ease);
  }
  .fresh button:hover {
    transform: translateY(-2px);
  }
  .fresh :global(.cover) {
    width: 56px;
    height: 56px;
  }
  .quiet,
  .foot {
    margin: 22px 0 0 4px;
    font-size: 13px;
    color: var(--mute);
  }
  .foot {
    margin-top: 28px;
    font-size: 12px;
    color: var(--dim);
  }
  .empty {
    max-width: 52ch;
    margin: 28px 0 0 4px;
  }
  .empty b {
    font-family: var(--font-display);
    font-size: 19px;
  }
  .empty p {
    margin-top: 6px;
    font-size: 13.5px;
    color: var(--mute);
  }
</style>
