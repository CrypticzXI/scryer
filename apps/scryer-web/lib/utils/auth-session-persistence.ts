export type AuthSessionPersistence = "persistent" | "tab";

export const SESSION_STORAGE_KEY = "scryer_auth_token";
export const PERSISTENT_STORAGE_KEY = "scryer_auth_token_persistent";

export function authSessionPersistence(
  persistSession: boolean,
): AuthSessionPersistence {
  return persistSession ? "persistent" : "tab";
}

export function storeAuthToken(token: string, persistSession: boolean) {
  sessionStorage.removeItem(SESSION_STORAGE_KEY);
  localStorage.removeItem(PERSISTENT_STORAGE_KEY);
  if (authSessionPersistence(persistSession) === "persistent") {
    localStorage.setItem(PERSISTENT_STORAGE_KEY, token);
  } else {
    sessionStorage.setItem(SESSION_STORAGE_KEY, token);
  }
}
