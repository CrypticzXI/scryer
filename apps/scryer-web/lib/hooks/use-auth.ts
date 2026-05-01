import { useCallback, useEffect, useRef, useState } from "react";
import { backendClient } from "@/lib/graphql/urql-client";
import { decodeJwtPayload, isTokenExpired } from "@/lib/utils/jwt";
import { authRuntimeStateQuery, meQuery } from "@/lib/graphql/queries";
import { loginMutation } from "@/lib/graphql/mutations";
import type { AuthRuntimeState } from "@/lib/types/settings";

const SESSION_STORAGE_KEY = "scryer_auth_token";

type AuthUser = { id: string; username: string; entitlements: string[] };
type AuthLoginOptions = { persistSession?: boolean };
type AuthLoginResult = { token: string; user: AuthUser | null };

// Module-level token ref so getAuthToken() can be called outside React
let currentToken: string | null = null;

export function getAuthToken(): string | null {
  return currentToken;
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

function applyAuthenticatedSession(
  token: string,
  user: AuthUser | null,
  setToken: (value: string | null) => void,
  setUser: (value: AuthUser | null) => void,
) {
  persistAuthToken(token);
  setToken(token);
  setUser(user);
}

export type AuthState = {
  token: string | null;
  user: AuthUser | null;
  loading: boolean;
  effectiveFormLoginEnabled: boolean | null;
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
  if (!payload || isTokenExpired(payload)) return null;
  return {
    id: payload.sub,
    username: payload.username,
    entitlements: payload.entitlements,
  };
}

export function useAuth(): AuthState {
  const [token, setToken] = useState<string | null>(null);
  const [user, setUser] = useState<AuthUser | null>(null);
  const [loading, setLoading] = useState(true);
  const [effectiveFormLoginEnabled, setEffectiveFormLoginEnabled] = useState<boolean | null>(null);
  const initialized = useRef(false);

  useEffect(() => {
    if (initialized.current) return;
    initialized.current = true;

    (async () => {
      const loadUserFromBypass = async (options?: { clearToken?: boolean }) => {
        if (options?.clearToken) {
          clearPersistedAuthToken();
          setToken(null);
        }

        try {
          const { data, error } = await backendClient.query(meQuery, {}).toPromise();
          if (error) throw error;
          return data?.me ?? null;
        } catch {
          return null;
        }
      };

      let runtimeState: AuthRuntimeState | null = null;

      try {
        const { data, error } = await backendClient.query(authRuntimeStateQuery, {}).toPromise();
        if (error) throw error;
        runtimeState = data?.authRuntimeState ?? null;
        setEffectiveFormLoginEnabled(
          typeof runtimeState?.effectiveFormLoginEnabled === "boolean"
            ? runtimeState.effectiveFormLoginEnabled
            : null,
        );
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
        runtimeState?.effectiveFormLoginEnabled !== true ||
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
      input: { username, password },
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
    login,
    adoptSession,
    logout,
  };
}
