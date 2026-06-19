import { useCallback, useEffect, useRef, useState } from "react";
import { backendClient } from "@/lib/graphql/urql-client";
import { decodeJwtPayload, isTokenExpired } from "@/lib/utils/jwt";
import { authRuntimeStateQuery, meQuery } from "@/lib/graphql/queries";
import { loginMutation } from "@/lib/graphql/mutations";
import type { AuthRuntimeState } from "@/lib/types/settings";
import type { UserAccountKind } from "@/lib/types/users";
import type { AppPermission, LibraryPermissionGrant } from "@/lib/utils/permissions";

const SESSION_STORAGE_KEY = "scryer_auth_token";

export type AuthUser = {
  id: string;
  username: string;
  hasPassword?: boolean;
  hasMfa?: boolean;
  hasPasskey?: boolean;
  accountKind?: UserAccountKind;
  appPermissions: AppPermission[];
  libraryPermissions: LibraryPermissionGrant[];
};
type AuthLoginOptions = { persistSession?: boolean; totpCode?: string | null };
type AuthLoginResult = { token: string; user: AuthUser | null; mfaEnrollmentRequired: boolean };

// Module-level token ref so getAuthToken() can be called outside React
let currentToken: string | null = null;

export function getAuthToken(): string | null {
  if (currentToken && userFromToken(currentToken)) {
    return currentToken;
  }

  currentToken = null;
  if (typeof window === "undefined") {
    return null;
  }

  const stored = window.sessionStorage.getItem(SESSION_STORAGE_KEY);
  if (!stored) {
    return null;
  }

  if (!userFromToken(stored)) {
    window.sessionStorage.removeItem(SESSION_STORAGE_KEY);
    return null;
  }

  currentToken = stored;
  return stored;
}

export function getMfaEnrollmentToken(): string | null {
  if (currentToken && isMfaEnrollmentToken(currentToken)) {
    return currentToken;
  }

  if (typeof window === "undefined") {
    return null;
  }

  const stored = window.sessionStorage.getItem(SESSION_STORAGE_KEY);
  if (!stored || !isMfaEnrollmentToken(stored)) {
    return null;
  }

  currentToken = stored;
  return stored;
}

export function clearClientAuthSession() {
  clearPersistedAuthToken();
}

function persistAuthToken(token: string) {
  sessionStorage.setItem(SESSION_STORAGE_KEY, token);
  currentToken = token;
}

function clearPersistedAuthToken() {
  sessionStorage.removeItem(SESSION_STORAGE_KEY);
  currentToken = null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value != null && typeof value === "object";
}

function isRateLimitedError(error: unknown): boolean {
  if (!isRecord(error) || !Array.isArray(error.graphQLErrors)) {
    return false;
  }

  return error.graphQLErrors.some((entry) => {
    if (!isRecord(entry)) {
      return false;
    }

    const extensions = isRecord(entry.extensions) ? entry.extensions : null;
    if (extensions?.code === "RATE_LIMITED") {
      return true;
    }

    return (
      typeof entry.message === "string" &&
      entry.message.trim().toLowerCase() === "api rate limit exceeded"
    );
  });
}

async function waitForRateLimitWindow(attempt: number) {
  const delayMs = [250, 500, 1_000][Math.min(attempt, 2)];
  await new Promise((resolve) => window.setTimeout(resolve, delayMs));
}

function applyAuthenticatedSession(
  token: string,
  user: AuthUser | null,
  setToken: (value: string | null) => void,
  setUser: (value: AuthUser | null) => void,
) {
  persistAuthToken(token);
  setToken(token);
  setUser(isMfaEnrollmentToken(token) ? null : user);
}

export type AuthState = {
  token: string | null;
  user: AuthUser | null;
  loading: boolean;
  effectiveFormLoginEnabled: boolean | null;
  passkeyEnabled: boolean;
  mfaRequirePasswordLogin: boolean;
  totpRequireJellyfinLogin: boolean;
  login: (
    username: string,
    password: string,
    options?: AuthLoginOptions,
  ) => Promise<AuthLoginResult>;
  adoptSession: (token: string, user: AuthUser | null) => void;
  logout: () => void;
};

/** Extract AuthUser from a JWT payload, or null if the token is invalid/expired. */
function userFromToken(token: string): AuthUser | null {
  const payload = decodeJwtPayload(token);
  if (!payload || isTokenExpired(payload) || payload.authScope === "mfa_enrollment") return null;
  return {
    id: payload.sub,
    username: payload.username,
    appPermissions: (payload.appPermissions ?? []) as AppPermission[],
    libraryPermissions: (payload.libraryPermissions ?? []) as LibraryPermissionGrant[],
  };
}

function isMfaEnrollmentToken(token: string): boolean {
  const payload = decodeJwtPayload(token);
  return Boolean(payload && !isTokenExpired(payload) && payload.authScope === "mfa_enrollment");
}

export function useAuth(): AuthState {
  const [token, setToken] = useState<string | null>(null);
  const [user, setUser] = useState<AuthUser | null>(null);
  const [loading, setLoading] = useState(true);
  const [effectiveFormLoginEnabled, setEffectiveFormLoginEnabled] = useState<boolean | null>(null);
  const [passkeyEnabled, setPasskeyEnabled] = useState(false);
  const [mfaRequirePasswordLogin, setMfaRequirePasswordLogin] = useState(false);
  const [totpRequireJellyfinLogin, setTotpRequireJellyfinLogin] = useState(false);
  const initialized = useRef(false);

  useEffect(() => {
    if (initialized.current) return;
    initialized.current = true;

    (async () => {
      const queryWithRateLimitRetry = async <TData>(
        query: string,
      ): Promise<TData | null> => {
        for (let attempt = 0; attempt < 3; attempt += 1) {
          const { data, error } = await backendClient.query(query, {}).toPromise();
          if (!error) {
            return (data as TData | null) ?? null;
          }
          if (!isRateLimitedError(error) || attempt === 2) {
            throw error;
          }
          await waitForRateLimitWindow(attempt);
        }

        return null;
      };

      const loadUserFromBypass = async (options?: { clearToken?: boolean }) => {
        if (options?.clearToken) {
          clearPersistedAuthToken();
          setToken(null);
        }

        try {
          const data = await queryWithRateLimitRetry<{ me?: AuthUser | null }>(meQuery);
          return data?.me ?? null;
        } catch {
          return null;
        }
      };

      let runtimeState: AuthRuntimeState | null = null;

      try {
        const data = await queryWithRateLimitRetry<{ authRuntimeState?: AuthRuntimeState | null }>(
          authRuntimeStateQuery,
        );
        runtimeState = data?.authRuntimeState ?? null;
        setEffectiveFormLoginEnabled(
          typeof runtimeState?.effectiveFormLoginEnabled === "boolean"
            ? runtimeState.effectiveFormLoginEnabled
            : null,
        );
        setPasskeyEnabled(runtimeState?.passkeyEnabled === true);
        setMfaRequirePasswordLogin(runtimeState?.mfaRequirePasswordLogin === true);
        setTotpRequireJellyfinLogin(runtimeState?.totpRequireJellyfinLogin === true);
      } catch {
        // Fall back to the existing token/bootstrap path when the public
        // runtime-state probe is temporarily unavailable.
      }

      if (runtimeState?.effectiveFormLoginEnabled === false) {
        clearPersistedAuthToken();
        setToken(null);
        setUser(await loadUserFromBypass());

        setLoading(false);
        return;
      }

      // When auth is enabled, or the runtime mode is temporarily unknown,
      // prefer preserving an existing valid session over clearing it.
      if (currentToken) {
        const authUser = userFromToken(currentToken);
        if (authUser) {
          setToken(currentToken);
          setUser(authUser);
          setLoading(false);
          return;
        }
        currentToken = null;
      }

      const stored = sessionStorage.getItem(SESSION_STORAGE_KEY);
      if (stored) {
        const authUser = userFromToken(stored);
        if (authUser) {
          currentToken = stored;
          setToken(stored);
          setUser(authUser);
          setLoading(false);
          return;
        }
        sessionStorage.removeItem(SESSION_STORAGE_KEY);
      }

      if (
        runtimeState == null ||
        runtimeState?.skipLoginForLocalIps === true
      ) {
        let bypassUser = await loadUserFromBypass();
        if (
          !bypassUser &&
          runtimeState?.skipLoginForLocalIps === true &&
          currentToken
        ) {
          bypassUser = await loadUserFromBypass({ clearToken: true });
        }
        setUser(bypassUser);
      }

      setLoading(false);
    })();
  }, []);

  const login = useCallback(async (
    username: string,
    password: string,
    options?: AuthLoginOptions,
  ) => {
    const { data, error } = await backendClient.mutation(loginMutation, {
      input: { username, password, totpCode: options?.totpCode ?? null },
    }).toPromise();
    if (error || !data?.login) {
      throw error ?? new Error("Login failed");
    }
    const newToken = data.login.token;
    const nextUser = data.login.user ?? userFromToken(newToken);

    if (options?.persistSession !== false) {
      applyAuthenticatedSession(newToken, nextUser, setToken, setUser);
    }

    return {
      token: newToken,
      user: nextUser,
      mfaEnrollmentRequired: data.login.mfaEnrollmentRequired === true,
    };
  }, []);

  const adoptSession = useCallback((nextToken: string, nextUser: AuthUser | null) => {
    applyAuthenticatedSession(nextToken, nextUser, setToken, setUser);
  }, []);

  const logout = useCallback(() => {
    clearPersistedAuthToken();
    setToken(null);
    setUser(null);
  }, []);

  return {
    token,
    user,
    loading,
    effectiveFormLoginEnabled,
    passkeyEnabled,
    mfaRequirePasswordLogin,
    totpRequireJellyfinLogin,
    login,
    adoptSession,
    logout,
  };
}
