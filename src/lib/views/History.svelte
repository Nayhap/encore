<script lang="ts">
  import { call, type BackendError, type HistoryGroup, type HistoryPage } from "$lib/backend";
  import { catalog } from "$lib/catalog.svelte";
  import { Resource } from "$lib/load.svelte";
  import { account, nav, openExternal, ui } from "$lib/state.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import Status from "$lib/components/Status.svelte";
  import TrackTable from "$lib/components/TrackTable.svelte";

  // Your YouTube Music history, grouped by day as YouTube groups it (D19).
  // Not cached: it changes with every play.
  const ACTIVITY = "https://myactivity.google.com/product/youtube";

  let groups = $state<HistoryGroup[]>([]);
  let cursor = $state<string | null>(null);
  let loadingMore = $state(false);
  const res = new Resource<HistoryPage>(async () => {
    const p = await call((a) => a.history());
    for (const g of p.groups) catalog.addTracks(g.rows.map((r) => r.track));
    groups = p.groups;
    cursor = p.cursor;
    return p;
  }, false);
  const page = $derived(res.data);

  $effect(() => {
    account.state.state;
    if (account.signedIn) void res.reload();
  });
  // A play was just reported: refresh (at most every few seconds).
  let lastRev = ui.historyRev;
  let timer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    const r = ui.historyRev;
    if (r === lastRev) return;
    lastRev = r;
    clearTimeout(timer);
    timer = setTimeout(() => void res.reload(), 2500);
    return () => clearTimeout(timer);
  });

  async function more() {
    if (!cursor || loadingMore) return;
    loadingMore = true;
    try {
      const p = await call((a) => a.historyMore(cursor!));
      const next = [...groups];
      for (const g of p.groups) {
        catalog.addTracks(g.rows.map((r) => r.track));
        // Rows without a title continue the last day.
        if (!g.title && next.length) next[next.length - 1] = { ...next[next.length - 1], rows: [...next[next.length - 1].rows, ...g.rows] };
        else next.push(g);
      }
      groups = next;
      cursor = p.cursor;
    } catch (e) {
      ui.say(`Could not load more: ${(e as BackendError).message}`);
    } finally {
      loadingMore = false;
    }
  }

  async function remove(gi: number, i: number) {
    const row = groups[gi]?.rows[i];
    if (!row) return;
    const before = groups;
    groups = groups.map((g, k) => (k === gi ? { ...g, rows: g.rows.filter((_, j) => j !== i) } : g)).filter((g) => g.rows.length);
    try {
      const r = await call((a) => a.historyRemove([row.rowId]));
      if (r === "done") ui.say(`Removed “${row.track.title}” from history`);
      else if (r === "rejected") {
        groups = before;
        ui.say("YouTube Music did not remove that. Try again in a moment.");
      } else {
        ui.say("Could not confirm the removal; reloading your history");
        void res.reload();
      }
    } catch (e) {
      groups = before;
      ui.say(`Could not remove: ${(e as BackendError).message}`);
    }
  }
</script>

<div class="page">
  <h1 class="page-title">History</h1>

  {#if !account.signedIn}
    <Status empty="Sign in to see what you've played on YouTube Music." />
    <button class="pill primary" onclick={() => nav.go({ name: "settings" })}>Sign in</button>
  {:else}
    {#if page?.paused || page?.notice}
      <div class="notice" role="status">
        <Icon name="history" size={18} />
        <div>
          <b>{page.paused ? "Your YouTube history is paused" : page.notice}</b>
          {#if page.paused}<p>New plays won't show up here until you turn it back on in your Google account.</p>{/if}
        </div>
        <button class="pill" onclick={() => openExternal(ACTIVITY)}>Manage history <Icon name="external" size={14} /></button>
      </div>
    {:else if !ui.reportHistory}
      <p class="off">Plays in Encore aren't sent to your history. <button onclick={() => nav.go({ name: "settings" })}>Change in Settings</button></p>
    {/if}

    <Status loading={res.loading && !groups.length} error={res.error} onretry={() => res.reload()} empty={!res.loading && !res.error && !groups.length && !page?.paused ? "Nothing played yet. Songs you play show up here." : null} />

    {#each groups as g, gi (gi + ":" + g.title)}
      <section class="day">
        <h2>{g.title}</h2>
        <TrackTable
          tracks={g.rows.map((r) => r.track)}
          label="History"
          head={gi === 0}
          extraMenu={(i) => (g.rows[i]?.removable ? [{ label: "Remove from history", icon: "x", danger: true, run: () => remove(gi, i) }] : [])}
        />
      </section>
    {/each}

    {#if cursor}
      <button class="pill more" disabled={loadingMore} onclick={more}>{loadingMore ? "Loading…" : "Show more"}</button>
    {/if}
  {/if}
</div>

<style>
  .day {
    margin-top: 22px;
  }
  .day:first-of-type {
    margin-top: 18px;
  }
  h2 {
    font-family: var(--font-display);
    font-size: 19px;
    font-weight: 650;
    letter-spacing: -0.01em;
    margin: 0 0 8px 8px;
  }
  .notice {
    display: flex;
    align-items: center;
    gap: 14px;
    margin-top: 16px;
    padding: 12px 14px 12px 16px;
    border-radius: 12px;
    background: color-mix(in srgb, var(--accent) 12%, var(--panel));
    font-size: 13.5px;
  }
  .notice > div {
    flex: 1;
    min-width: 0;
  }
  .notice p {
    color: var(--mute);
    font-size: 13px;
  }
  .notice :global(svg) {
    color: var(--accent);
  }
  .notice .pill :global(svg) {
    color: inherit;
  }
  .off {
    margin-top: 12px;
    font-size: 13px;
    color: var(--mute);
  }
  .off button {
    color: var(--text);
    font-weight: 550;
    text-decoration: underline;
    text-underline-offset: 3px;
  }
  .more {
    margin: 20px 0 0 8px;
  }
</style>
