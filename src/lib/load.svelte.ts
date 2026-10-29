// A small async resource for views: loading, data, error, reload.
import { BackendError } from "./backend";

export class Resource<T> {
  data = $state<T | null>(null);
  error = $state<BackendError | null>(null);
  loading = $state(true);
  #fn: () => Promise<T>;
  #seq = 0;

  constructor(fn: () => Promise<T>, start = true) {
    this.#fn = fn;
    if (start) void this.reload();
  }

  async reload() {
    const seq = ++this.#seq;
    this.loading = true;
    this.error = null;
    try {
      const d = await this.#fn();
      if (seq !== this.#seq) return;
      this.data = d;
    } catch (e) {
      if (seq !== this.#seq) return;
      this.error = e instanceof BackendError ? e : new BackendError({ code: "internal", message: String(e) });
    } finally {
      if (seq === this.#seq) this.loading = false;
    }
  }
}

/** What to tell the user for an error, and what they can do. */
export function explain(e: BackendError): { title: string; hint: string } {
  switch (e.code) {
    case "network":
      return { title: "Could not reach YouTube Music", hint: "Check your internet connection, then try again." };
    case "authRequired":
      return { title: "Sign in to see this", hint: "Open Settings → Account to sign in or import your session from a browser." };
    case "sessionExpired":
      return { title: "Your session expired", hint: "Open Settings → Account and import your session again." };
    case "challenged":
      return { title: "YouTube asked for a verification", hint: "Wait a minute and try again." };
    case "notFound":
      return { title: "Not found", hint: "This page may have been removed." };
    case "parse":
      return { title: "YouTube Music changed something", hint: "Encore could not read this page. Try again later or update Encore." };
    default:
      return { title: "Something went wrong", hint: e.message };
  }
}
