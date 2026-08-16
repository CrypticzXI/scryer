/**
 * Whether a Jellyfin account-link request is complete enough to submit.
 *
 * Password is intentionally absent. Jellyfin allows passwordless accounts, and
 * linking one is supported so its identity, watch data, and lists can be pulled.
 * An empty password only ever verifies against a genuinely passwordless account,
 * because Jellyfin returns 401 for an empty `Pw` on any account that has one.
 *
 * Refusing to *sign in* with a passwordless Jellyfin account is a separate
 * concern, enforced server-side in `federated_login_with_jellyfin`. Do not
 * reintroduce a password requirement here to try to cover that.
 */
export function canSubmitJellyfinLink({
  connectionId,
  username,
  busy,
}: {
  connectionId: string | null | undefined;
  username: string;
  busy: boolean;
}): boolean {
  return Boolean(connectionId) && username.trim().length > 0 && !busy;
}

export function effectiveEmbyLinkMode(
  requestedMode: "LOCAL" | "CONNECT",
  connectEnabled: boolean,
): "LOCAL" | "CONNECT" {
  return requestedMode === "CONNECT" && connectEnabled ? "CONNECT" : "LOCAL";
}
