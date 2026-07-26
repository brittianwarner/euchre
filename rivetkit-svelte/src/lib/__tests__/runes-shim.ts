const effect = ((fn?: () => unknown) => fn?.()) as unknown as {
  (fn?: () => unknown): unknown;
  root: (fn: () => void | (() => void)) => () => void;
};

effect.root = (fn) => {
  const cleanup = fn();
  return typeof cleanup === "function" ? cleanup : () => {};
};

const state = ((value?: unknown) => value) as unknown as typeof $state;
(state as unknown as { raw: <T>(value: T) => T }).raw = <T>(value: T) => value;

(globalThis as Record<string, unknown>).$state = state;
(globalThis as Record<string, unknown>).$effect = effect;
