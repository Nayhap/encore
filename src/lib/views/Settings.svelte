<script lang="ts">
  import { onMount } from "svelte";
  import { call, isTauri, type AccountChannel, type BrowserProfile, type CacheInfo, type ResolverStatus } from "$lib/backend";
  import { account, openExternal, player, ui, type Theme } from "$lib/state.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import { commands } from "$lib/backend/bindings";
  import { isMac, os, secretStore } from "$lib/platform";

  // What browser import can and can't do on this OS (D8).
  const importHelp =
    os === "macos"
      ? "Uses the YouTube session already signed in to your browser. For Edge, Chrome and Brave, macOS asks once to allow access to the browser's “Safe Storage” key in the Keychain; choose Always Allow."
      : os === "windows"
        ? "Uses the YouTube session already signed in to Firefox. Current Edge and Chrome lock their cookies while they run and protect them with app-bound encryption that other apps can't read, so importing from them usually fails. For those, use Sign in above."
        : "Uses the YouTube session already signed in to your browser. For Chrome, Chromium, Edge and Brave, Encore reads the browser's key from GNOME Keyring or KWallet, which may ask you to unlock it.";
  const browserNames: Record<string, string> = { edge: "Microsoft Edge", chrome: "Google Chrome", brave: "Brave", chromium: "Chromium", firefox: "Firefox", safari: "Safari", webview: "Encore sign-in" };
  const sourceName = (s: string) => browserNames[s.split(":")[0]] ?? s;

  const themes: { v: Theme; label: string; icon: string }[] = [
    { v: "system", label: "Match system", icon: "settings" },
    { v: "dark", label: "Dark", icon: "moon" },
    { v: "light", label: "Light", icon: "sun" },
  ];

  let profiles = $state<BrowserProfile[]>([]);
  let profile = $state("");
  let busy = $state<string | null>(null);
  let pickKey = $state<string | null>(null);
  let switching = $state(false);
  let channels = $state<AccountChannel[]>([]);
  let resolver = $state<ResolverStatus | null>(null);
  let cache = $state<CacheInfo | null>(null);
  let err = $state<string | null>(null);

  onMount(async () => {
    profiles = await call((a) => a.browserProfiles()).catch(() => []);
    // Windows: Firefox is the browser import can actually read (D8).
    const prefer = os === "windows" ? "firefox:" : "edge:";
    profile = profiles.find((p) => p.id.startsWith(prefer))?.id ?? profiles[0]?.id ?? "";
    resolver = (await call((a) => a.bootstrap()).catch(() => null))?.resolver ?? null;
    cache = await call((a) => a.cacheInfo()).catch(() => null);
  });

  const st = $derived(account.state);
  const shownChannels = $derived(st.state === "choosingChannel" ? st.channels : switching ? channels : []);
  $effect(() => {
    if (shownChannels.length && !shownChannels.some((c) => c.key === pickKey)) pickKey = shownChannels[0].key;
  });

  async function run<T>(what: string, f: () => Promise<T>) {
    busy = what;
    err = null;
    try {
      return await f();
    } catch (e: any) {
      err = e?.message ?? String(e);
      return null;
    } finally {
      busy = null;
    }
  }

  const importNow = () => run("import", async () => (account.state = await call((a) => a.importBrowser(profile))));
  const signIn = () => run("signin", () => call((a) => a.beginSignin()));
  async function choose() {
    if (!pickKey) return;
    const s = await run("select", () => call((a) => a.selectChannel(pickKey!)));
    if (s) {
      account.state = s;
      switching = false;
      ui.say(s.state === "signedIn" ? `Signed in as ${s.channel.name}` : "Channel saved");
    }
  }
  async function startSwitch() {
    const c = await run("channels", () => call((a) => a.reloadChannels()));
    if (c) {
      channels = c;
      switching = true;
    }
  }
  const signOut = () => run("signout", async () => {
    await call((a) => a.signOut());
    account.state = { state: "anonymous" };
    ui.say("Signed out");
  });
  function setReport(on: boolean) {
    ui.reportHistory = on;
    call((a) => a.setReportHistory(on)).catch((e) => {
      ui.reportHistory = !on;
      ui.say(`Could not change that: ${e.message}`);
    });
  }
  const setupResolver = () => run("resolver", async () => (resolver = await call((a) => a.resolverSetup())));
  const clear = (w: "audio" | "art" | "lyrics") =>
    run("cache", async () => {
      await call((a) => a.cacheClear(w));
      cache = await call((a) => a.cacheInfo());
    });
  function setTitlebar(on: boolean) {
    ui.systemTitlebar = on;
    localStorage.setItem("titlebar", on ? "system" : "encore");
    if (isTauri) commands.setSystemTitlebar(on, true).catch((e: any) => ui.say(`Could not change the title bar: ${e?.message ?? e}`));
  }
  const mb = (b: number) => `${(b / 1048576).toFixed(b > 1048576 * 10 ? 0 : 1)} MB`;
  const stageText: Record<string, string> = {
    opened: "Sign in in the window that opened. Encore waits until you are back on YouTube Music.",
    validating: "Checking the session…",
    chooseChannel: "Signed in. Choose the channel to use below.",
    rejected: "Google refused to sign in inside Encore. Use “Import from browser” instead.",
    cancelled: "Sign-in window closed.",
    failed: "Sign-in did not finish.",
    error: "The session could not be checked yet; finish signing in, or close the window.",
  };
</script>

<div class="page">
  <h1 class="page-title">Settings</h1>

  <section>
    <h2>Account</h2>
    {#if st.state === "signedIn"}
      <div class="row">
        <div>
          <b>Signed in as {st.channel.name}</b>
          <p>{st.channel.isBrand ? "Brand channel" : "Personal channel"}{st.channel.handle ? ` · ${st.channel.handle}` : ""} · from {sourceName(st.source)}</p>
        </div>
        <div class="btns">
          <button class="pill" disabled={!!busy} onclick={startSwitch}>Switch channel</button>
          <button class="pill" disabled={!!busy} onclick={signOut}>Sign out</button>
        </div>
      </div>
    {:else if st.state === "expired"}
      <div class="row">
        <div>
          <b>Session expired</b>
          <p>{st.reason} Imported sessions stop working when the browser rotates its cookies. Import again, or sign in inside Encore.</p>
        </div>
        <div class="btns">
          <button class="pill primary" disabled={!!busy || !isTauri} onclick={signIn}>{busy === "signin" ? "Opening…" : "Sign in"}</button>
          <button class="pill" disabled={!!busy} onclick={signOut}>Sign out</button>
        </div>
      </div>
    {:else if st.state === "anonymous"}
      <div class="row">
        <div>
          <b>YouTube Music</b>
          <p>Sign in to see your library, likes and playlists. Sign-in waits until you have picked an account, including brand accounts.</p>
        </div>
        <button class="pill primary" disabled={!!busy || !isTauri} onclick={signIn}>{busy === "signin" ? "Opening…" : "Sign in"}</button>
      </div>
    {/if}

    {#if st.state !== "signedIn"}
      <div class="row">
        <div>
          <b>Import from browser</b>
          <p>{importHelp} Your session is stored only in {secretStore}.</p>
        </div>
        <div class="btns">
          <select bind:value={profile} aria-label="Browser profile" disabled={!profiles.length}>
            {#each profiles as p}<option value={p.id}>{p.browser} — {p.name}</option>{/each}
            {#if !profiles.length}<option value="">No browsers found</option>{/if}
          </select>
          <button class="pill primary" disabled={!profile || !!busy} onclick={importNow}>{busy === "import" ? "Importing…" : "Import"}</button>
        </div>
      </div>
    {/if}

    {#if account.signin && stageText[account.signin.stage] && st.state !== "signedIn"}
      <p class="note">{stageText[account.signin.stage]}{account.signin.message ? ` ${account.signin.message}` : ""}</p>
    {/if}

    {#if shownChannels.length}
      <div class="picker" role="radiogroup" aria-label="Choose a channel">
        <p class="note">Choose the channel Encore should use. Brand channels have their own library and history.</p>
        {#each shownChannels as c (c.key)}
          <label class="chan">
            <input type="radio" name="chan" value={c.key} bind:group={pickKey} />
            <span><b>{c.name}</b>{#if c.handle}<small> {c.handle}</small>{/if}{#if c.isBrand}<small> · brand</small>{/if}</span>
          </label>
        {/each}
        <div class="btns">
          <button class="pill primary" disabled={!pickKey || !!busy} onclick={choose}>{busy === "select" ? "Checking…" : "Continue"}</button>
          {#if switching}<button class="pill" onclick={() => (switching = false)}>Cancel</button>{/if}
        </div>
      </div>
    {/if}
    {#if err}<p class="note error" role="alert">{err}</p>{/if}
  </section>

  <section>
    <h2>Appearance</h2>
    <div class="row">
      <div><b>Theme</b></div>
      <div class="seg" role="radiogroup" aria-label="Theme">
        {#each themes as t}
          <button role="radio" aria-checked={ui.theme === t.v} onclick={() => (ui.theme = t.v)}>
            <Icon name={t.icon} size={15} />{t.label}
          </button>
        {/each}
      </div>
    </div>
    <label class="row">
      <div>
        <b>Colour from artwork</b>
        <p>Tint highlights and backgrounds with colours from the song that is playing.</p>
      </div>
      <input type="checkbox" role="switch" bind:checked={ui.tint} />
    </label>
    {#if !isMac}
      <label class="row">
        <div>
          <b>Use the system title bar</b>
          <p>Show your desktop's own title bar and window buttons instead of Encore's.{os === "linux" ? " Encore restarts to apply it." : ""}</p>
        </div>
        <input type="checkbox" role="switch" checked={ui.systemTitlebar} onchange={(e) => setTitlebar(e.currentTarget.checked)} />
      </label>
    {/if}
  </section>

  <section>
    <h2>Playback</h2>
    <label class="row">
      <div>
        <b>Autoplay</b>
        <p>Keep playing similar songs when the queue ends.</p>
      </div>
      <input type="checkbox" role="switch" checked={player.autoplay} onchange={(e) => player.setAutoplay(e.currentTarget.checked)} />
    </label>
    <div class="row">
      <label class="grow" for="report-history">
        <b>Send plays to YouTube history</b>
        <p>
          {account.signedIn ? "Songs you play here show up in your YouTube Music history and shape your recommendations." : "When you're signed in, songs you play here show up in your YouTube Music history."}
          YouTube's own history setting still applies.
        </p>
        <button type="button" class="link" onclick={() => openExternal("https://myactivity.google.com/product/youtube")}>Manage your YouTube history <Icon name="external" size={13} /></button>
      </label>
      <input id="report-history" type="checkbox" role="switch" checked={ui.reportHistory} onchange={(e) => setReport(e.currentTarget.checked)} />
    </div>
    <div class="row">
      <div>
        <b>Stream resolver</b>
        <p>
          {#if resolver?.ready}Ready · yt-dlp {resolver.ytDlp} · Python {resolver.python ?? "?"} · {resolver.jsRuntime ?? "no JS runtime"}
          {:else if resolver?.message}{resolver.message}
          {:else}Set up on first play (creates a private Python environment with yt-dlp).{/if}
        </p>
      </div>
      {#if !resolver?.ready}<button class="pill" disabled={!!busy} onclick={setupResolver}>{busy === "resolver" ? "Setting up…" : "Set up now"}</button>{/if}
    </div>
  </section>

  <section>
    <h2>Storage</h2>
    <div class="row">
      <div>
        <b>Caches</b>
        <p>Audio {cache ? mb(cache.audioBytes ?? 0) : "–"} · Artwork {cache ? mb(cache.artBytes ?? 0) : "–"} · Lyrics {cache?.lyricsEntries ?? "–"} songs</p>
      </div>
      <div class="btns">
        <button class="pill" disabled={!!busy} onclick={() => clear("audio")}>Clear audio</button>
        <button class="pill" disabled={!!busy} onclick={() => clear("art")}>Clear artwork</button>
        <button class="pill" disabled={!!busy} onclick={() => clear("lyrics")}>Clear lyrics</button>
      </div>
    </div>
  </section>
</div>

<style>
  section {
    max-width: 720px;
    margin-top: 28px;
  }
  h2 {
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0.07em;
    text-transform: uppercase;
    color: var(--dim);
    margin-bottom: 6px;
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 24px;
    padding: 14px 0;
    border-bottom: 1px solid var(--line);
  }
  .row p {
    margin-top: 2px;
    font-size: 13px;
    color: var(--mute);
    max-width: 52ch;
  }
  .seg {
    display: flex;
    padding: 3px;
    gap: 2px;
    border-radius: 10px;
    background: var(--hover);
  }
  .seg button {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    padding: 0 12px;
    border-radius: 8px;
    font-size: 13px;
    font-weight: 550;
    color: var(--mute);
  }
  .seg button[aria-checked="true"] {
    background: var(--panel);
    color: var(--text);
    box-shadow: 0 1px 3px rgb(0 0 0 / 0.2);
  }
  .btns {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
    justify-content: flex-end;
  }
  select {
    height: 34px;
    max-width: 240px;
    padding: 0 10px;
    border-radius: 999px;
    border: 1px solid var(--line);
    background: var(--hover);
    color: var(--text);
    font: inherit;
    font-size: 13px;
  }
  .grow {
    display: grid;
    justify-items: start;
  }
  .link {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    margin-top: 6px;
    font-size: 13px;
    font-weight: 550;
    color: var(--text);
  }
  .link:hover {
    text-decoration: underline;
    text-underline-offset: 3px;
  }
  .note {
    margin-top: 10px;
    font-size: 13px;
    color: var(--mute);
  }
  .note.error {
    color: hsl(0 70% 62%);
  }
  .picker {
    display: grid;
    gap: 8px;
    padding: 14px 0;
    border-bottom: 1px solid var(--line);
  }
  .chan {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    border-radius: 10px;
    background: var(--hover);
  }
  .chan small {
    color: var(--mute);
  }
  .picker .btns {
    justify-content: flex-start;
  }
</style>
