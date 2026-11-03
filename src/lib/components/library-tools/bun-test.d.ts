// The part of Bun's test runner logic.test.ts uses, so svelte-check knows
// it without bun-types as a dependency.
declare module "bun:test" {
  type Fn = () => void | Promise<void>;
  export function describe(name: string, f: Fn): void;
  export function test(name: string, f: Fn): void;
  interface Matchers {
    toBe(v: unknown): void;
    toEqual(v: unknown): void;
    toBeNull(): void;
  }
  export function expect(v: unknown): Matchers;
}
