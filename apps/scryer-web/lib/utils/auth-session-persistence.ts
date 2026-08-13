export type AuthSessionPersistence = "persistent" | "tab";

export function authSessionPersistence(
  persistSession: boolean,
): AuthSessionPersistence {
  return persistSession ? "persistent" : "tab";
}
