<script lang="ts">
  // Settings → Updates (D24): one line for the state, the buttons that go
  // with it, and the automatic switch.
  import { onMount } from "svelte";
  import { openExternal, ui, updates } from "$lib/state.svelte";
  import { isMac } from "$lib/platform";

  const up = $derived(updates.status);
  const avail = $derived(up?.available ?? null);
  const skippedNow = $derived(!!avail && up?.skipped === avail.version);
  let now = $state(Date.now());
  onMount(() => {
    const t = setInterval(() => (now = Date.now()), 30_000);
    return () => clearInterval(t);
  });
  function ago(t: number) {
    const m = Math.round((now / 1000 - t) / 60);
    if (m < 1) return "just now";
    if (m < 60) return `${m} minute${m === 1 ? "" : "s"} ago`;
    const h = Math.round(m / 60);
    if (h < 24) return `${h} hour${h === 1 ? "" : "s"} ago`;
    const d = Math.round(h / 24);
    return `${d} day${d === 1 ? "" : "s"} ago`;
  }
  const updateLine = $derived.by(() => {
    if (!up) return "Checking…";
    const v = avail?.version;
    switch (up.phase) {
      case "checking":
        return "Checking for updates…";
      case "downloading":
        return `Downloading Encore ${v}${up.total ? ` · ${Math.floor(((up.received ?? 0) / up.total) * 100)}%` : "…"}`;
      case "ready":
        return up.error ?? (up.atQuit ? `Encore ${v} is ready. It installs when you quit Encore.` : `Encore ${v} is ready to install.`);
      case "installing":
        return "Installing the update…";
    }
    if (up.install === "dev") return up.reason ?? "";
    if (skippedNow) return `Encore ${v} was skipped.`;
    if (avail) {
      if (up.phase === "failed" && up.error) return `Couldn't download Encore ${v}: ${up.error}. Encore tries again later.`;
      return up.canInstall ? `Encore ${v} is available.` : `Encore ${v} is out. ${up.reason ?? ""}`;
    }
    if (up.error) return `Couldn't check for updates: ${up.error}. Encore tries again later.`;
    if (!ui.online && up.phase !== "upToDate") return "You're offline. Encore checks again when you're back online.";
    if (up.checkedAt) return `Up to date · checked ${ago(up.checkedAt)}`;
    return "Encore checks for updates by itself.";
  });
</script>

<section>
  <h2>Updates</h2>
  <div class="row">
    <div>
      <b>Encore {up?.current ?? ""}</b>
      <p>{updateLine}</p>
      {#if up && !up.canInstall && up.install !== "dev" && !avail && up.reason}<p>{up.reason}</p>{/if}
    </div>
    <div class="btns">
      {#if up?.phase === "ready"}
        <button class="pill primary" onclick={() => updates.install()}>Restart to update</button>
      {/if}
      {#if avail && up?.phase !== "ready"}
        {#if skippedNow}
          <button class="pill" onclick={() => updates.unskip()}>Show it again</button>
        {:else if !up?.canInstall}
          <button class="pill primary" onclick={() => openExternal(avail.page)}>Open releases page</button>
        {:else if up?.phase === "available" || up?.phase === "failed"}
          <button class="pill primary" onclick={() => updates.download()}>Download</button>
        {/if}
      {/if}
      {#if avail}
        <button class="pill" onclick={() => (updates.sheet = "update")}>What's new</button>
      {/if}
      {#if up && up.install !== "dev" && !["checking", "downloading", "installing", "ready"].includes(up.phase)}
        <button class="pill" onclick={() => updates.check()}>Check for updates</button>
      {/if}
    </div>
  </div>
  {#if up && up.canInstall}
    <label class="row">
      <div>
        <b>Install updates automatically</b>
        <p>
          Encore checks GitHub for new versions a few times a day, downloads them in the background (20 to 100 MB) and installs them the next time you quit or restart it. Off, it only tells you when a new version is out.
          {#if isMac}macOS may ask once for access to your saved sign-in after an update.{/if}
        </p>
      </div>
      <input type="checkbox" role="switch" checked={up.auto} onchange={(e) => updates.setAuto(e.currentTarget.checked)} />
    </label>
  {/if}
</section>
