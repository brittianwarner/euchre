/**
 * Actor Error Utilities
 *
 * Helpers for inspecting errors from actor actions. The `@rivetkit/svelte`
 * action middleware normalizes all errors to `Error` instances, but at runtime
 * errors from `UserError` throws arrive as `ActorError` from `rivetkit/client`
 * with `.code`, `.group`, and `.metadata` intact.
 *
 * These utilities let consumers discriminate `ActorError` from generic `Error`
 * without importing `rivetkit/client` directly.
 *
 * @module
 */

/**
 * Type guard: is the error an `ActorError` from rivetkit/client?
 *
 * `ActorError` sets `__type = "ActorError"` as a discriminator,
 * which survives serialization boundaries and avoids `instanceof`
 * issues across package boundaries.
 */
export function isActorError(err: unknown): err is Error & {
  group: string;
  code: string;
  metadata?: unknown;
  __type: "ActorError";
} {
  return (
    err instanceof Error &&
    "__type" in err &&
    (err as Record<string, unknown>).__type === "ActorError"
  );
}

/**
 * Extract the machine-readable error code from an error, if it's an ActorError.
 * Returns `undefined` for non-ActorError instances.
 */
export function actorErrorCode(
  err: Error | null | undefined,
): string | undefined {
  if (!err) return undefined;
  if (isActorError(err)) return err.code;
  return undefined;
}

/**
 * Extract the human-readable error message from an error.
 * Works for both `ActorError` (message = UserError's first argument)
 * and generic `Error` instances.
 *
 * Returns `undefined` for null/undefined input.
 */
export function actorErrorMessage(
  err: Error | null | undefined,
): string | undefined {
  if (!err) return undefined;
  return err.message || undefined;
}

/**
 * Structured error info extracted from an actor handle's `lastActionError`.
 */
export interface ActionErrorInfo {
  /** Human-readable error message. */
  message: string | undefined;
  /** Machine-readable error code (only present for `ActorError`). */
  code: string | undefined;
  /** Whether this is an `ActorError` (from a `UserError` throw on the server). */
  isActorError: boolean;
}

/**
 * Extract structured error info from any actor handle's `lastActionError`.
 *
 * Designed for lazy connections where `BaseActorViewModel.actorErrorMessage`
 * is not available. Returns `null` when there is no error.
 *
 * @example
 * ```typescript
 * const error = getActionError(threadHandle);
 * if (error) {
 *   showToast(error.message ?? "Something went wrong");
 *   if (error.code === "RATE_LIMITED") retryLater();
 * }
 * ```
 */
export function getActionError(
  handle: { lastActionError: Error | null | undefined },
): ActionErrorInfo | null {
  const err = handle.lastActionError;
  if (!err) return null;
  return {
    message: actorErrorMessage(err),
    code: actorErrorCode(err),
    isActorError: isActorError(err),
  };
}
