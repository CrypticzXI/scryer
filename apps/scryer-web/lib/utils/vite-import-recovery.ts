const DYNAMIC_IMPORT_FAILURE = "Failed to fetch dynamically imported module";

export const VITE_IMPORT_RECOVERY_WINDOW_MS = 5_000;

export function shouldRetryStaleViteImport(
  error: unknown,
  previousAttemptAt: number | null,
  now: number,
): boolean {
  if (
    !(error instanceof Error) ||
    !error.message.includes(DYNAMIC_IMPORT_FAILURE)
  ) {
    return false;
  }

  return (
    previousAttemptAt === null ||
    previousAttemptAt + VITE_IMPORT_RECOVERY_WINDOW_MS < now
  );
}
