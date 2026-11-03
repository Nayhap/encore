// What the Settings sections share: one action at a time across the page,
// and the error from the last one (shown under Account).

class SettingsRun {
  busy = $state<string | null>(null);
  err = $state<string | null>(null);

  async run<T>(what: string, f: () => Promise<T>) {
    this.busy = what;
    this.err = null;
    try {
      return await f();
    } catch (e: any) {
      this.err = e?.message ?? String(e);
      return null;
    } finally {
      this.busy = null;
    }
  }
}

export const settings = new SettingsRun();

export const mb = (b: number) => `${(b / 1048576).toFixed(b > 1048576 * 10 ? 0 : 1)} MB`;
