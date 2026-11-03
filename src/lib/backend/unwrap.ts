// Turn a generated command's result into a value or a thrown BackendError.
import { BackendError } from "./index";

type R<T> = { status: "ok"; data: T } | { status: "error"; error: any };

export async function u<T>(p: Promise<R<T>>): Promise<T> {
  const r = await p;
  if (r.status === "error") throw new BackendError(r.error);
  return r.data;
}
