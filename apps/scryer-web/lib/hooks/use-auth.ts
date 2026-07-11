import { useCallback, useEffect, useRef, useState } from "react";
import { backendClient } from "@/lib/graphql/urql-client";
import { decodeJwtPayload, isTokenExpired } from "@/lib/utils/jwt";
import { authRuntimeStateQuery, meQuery } from "@/lib/graphql/queries";
import { loginMutation } from "@/lib/graphql/mutations";
import type { AuthRuntimeState } from "@/lib/types/settings";
import type { UserAccountKind } from "@/lib/types/users";
import {
  normalizeJwtPermissionClaims,
  type AppPermission,
  type LibraryPermissionGrant,
} from "@/lib/utils/permissions";

const SESSION_STORAGE_KEY = "scryer_auth_token";
export const AUTH_SESSION_CHANGED_EVENT = "scryer:auth-session-changed";

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
    clearPersistedAuthToken();
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

function dispatchAuthSessionChanged() {
  if (typeof window === "undefined") {
    return;
  }
  window.dispatchEvent(new CustomEvent(AUTH_SESSION_CHANGED_EVENT));
}

function persistAuthToken(token: string) {
  sessionStorage.setItem(SESSION_STORAGE_KEY, token);
  currentToken = token;
  dispatchAuthSessionChanged();
}

function clearPersistedAuthToken() {
  const hadCurrentToken = currentToken !== null;
  const hadPersistedToken =
    typeof window !== "undefined" &&
    sessionStorage.getItem(SESSION_STORAGE_KEY) !== null;

  if (typeof window !== "undefined") {
    sessionStorage.removeItem(SESSION_STORAGE_KEY);
  }
  currentToken = null;
  if (hadCurrentToken || hadPersistedToken) {
    dispatchAuthSessionChanged();
  }
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

type AuthBootstrapSnapshot = {
  token: string | null;
  user: AuthUser | null;
  effectiveFormLoginEnabled: boolean | null;
  passkeyEnabled: boolean;
  mfaRequirePasswordLogin: boolean;
  mfaRequireConfigStepUp: boolean | null;
  totpRequireJellyfinLogin: boolean;
};

let authBootstrapSnapshot: AuthBootstrapSnapshot | null = null;
let authBootstrapPromise: Promise<AuthBootstrapSnapshot> | null = null;

async function queryWithRateLimitRetry<TData>(
  query: string,
): Promise<TData | null> {
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
}

function normalizeAuthUser(user: AuthUser | null | undefined): AuthUser | null {
  if (!user) {
    return null;
  }

  return {
    ...user,
    ...normalizeJwtPermissionClaims(
      user.appPermissions,
      user.libraryPermissions,
    ),
  };
}

async function loadUserFromBypass(options?: { clearToken?: boolean }) {
  if (options?.clearToken) {
    clearPersistedAuthToken();
  }

  try {
    const data = await queryWithRateLimitRetry<{ me?: AuthUser | null }>(
      meQuery,
    );
    return normalizeAuthUser(data?.me);
  } catch {
    return null;
  }
}

function rememberAuthBootstrapSession(token: string | null, user: AuthUser | null) {
  authBootstrapPromise = null;
  authBootstrapSnapshot = {
    token,
    user,
    effectiveFormLoginEnabled:
      authBootstrapSnapshot?.effectiveFormLoginEnabled ?? null,
    passkeyEnabled: authBootstrapSnapshot?.passkeyEnabled ?? false,
    mfaRequirePasswordLogin:
      authBootstrapSnapshot?.mfaRequirePasswordLogin ?? false,
    mfaRequireConfigStepUp:
      authBootstrapSnapshot?.mfaRequireConfigStepUp ?? null,
    totpRequireJellyfinLogin:
      authBootstrapSnapshot?.totpRequireJellyfinLogin ?? false,
  };
}

async function computeAuthBootstrapSnapshot(): Promise<AuthBootstrapSnapshot> {
  let runtimeState: AuthRuntimeState | null = null;
  let effectiveFormLoginEnabled: boolean | null = null;
  let passkeyEnabled = false;
  let mfaRequirePasswordLogin = false;
  let mfaRequireConfigStepUp: boolean | null = null;
  let totpRequireJellyfinLogin = false;

  try {
    const data = await queryWithRateLimitRetry<{
      authRuntimeState?: AuthRuntimeState | null;
    }>(authRuntimeStateQuery);
    runtimeState = data?.authRuntimeState ?? null;
    effectiveFormLoginEnabled =
      typeof runtimeState?.effectiveFormLoginEnabled === "boolean"
        ? runtimeState.effectiveFormLoginEnabled
        : null;
    passkeyEnabled = runtimeState?.passkeyEnabled === true;
    mfaRequirePasswordLogin = runtimeState?.mfaRequirePasswordLogin === true;
    mfaRequireConfigStepUp =
      typeof runtimeState?.mfaRequireConfigStepUp === "boolean"
        ? runtimeState.mfaRequireConfigStepUp
        : null;
    totpRequireJellyfinLogin = runtimeState?.totpRequireJellyfinLogin === true;
  } catch {
    // Fall back to the existing token/bootstrap path when the public
    // runtime-state probe is temporarily unavailable.
  }

  if (runtimeState?.effectiveFormLoginEnabled === false) {
    clearPersistedAuthToken();
    return {
      token: null,
      user: await loadUserFromBypass(),
      effectiveFormLoginEnabled,
      passkeyEnabled,
      mfaRequirePasswordLogin,
      mfaRequireConfigStepUp,
      totpRequireJellyfinLogin,
    };
  }

  // When auth is enabled, or the runtime mode is temporarily unknown,
  // prefer preserving an existing valid session over clearing it.
  if (currentToken) {
    const authUser = userFromToken(currentToken);
    if (authUser) {
      return {
        token: currentToken,
        user: authUser,
        effectiveFormLoginEnabled,
        passkeyEnabled,
        mfaRequirePasswordLogin,
        mfaRequireConfigStepUp,
        totpRequireJellyfinLogin,
      };
    }
    clearPersistedAuthToken();
  }

  const stored = sessionStorage.getItem(SESSION_STORAGE_KEY);
  if (stored) {
    const authUser = userFromToken(stored);
    if (authUser) {
      currentToken = stored;
      return {
        token: stored,
        user: authUser,
        effectiveFormLoginEnabled,
        passkeyEnabled,
        mfaRequirePasswordLogin,
        mfaRequireConfigStepUp,
        totpRequireJellyfinLogin,
      };
    }
    clearPersistedAuthToken();
  }

  let user: AuthUser | null = null;
  if (runtimeState == null || runtimeState?.skipLoginForLocalIps === true) {
    user = await loadUserFromBypass();
    if (
      !user &&
      runtimeState?.skipLoginForLocalIps === true &&
      currentToken
    ) {
      user = await loadUserFromBypass({ clearToken: true });
    }
  }

  return {
    token: null,
    user,
    effectiveFormLoginEnabled,
    passkeyEnabled,
    mfaRequirePasswordLogin,
    mfaRequireConfigStepUp,
    totpRequireJellyfinLogin,
  };
}

function loadAuthBootstrapSnapshot(): Promise<AuthBootstrapSnapshot> {
  if (authBootstrapSnapshot) {
    return Promise.resolve(authBootstrapSnapshot);
  }
  if (!authBootstrapPromise) {
    authBootstrapPromise = computeAuthBootstrapSnapshot().then((snapshot) => {
      authBootstrapSnapshot = snapshot;
      authBootstrapPromise = null;
      return snapshot;
    });
  }
  return authBootstrapPromise;
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
  mfaRequireConfigStepUp: boolean | null;
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
  const authorization = normalizeJwtPermissionClaims(
    payload.appPermissions,
    payload.libraryPermissions,
  );
  return {
    id: payload.sub,
    username: payload.username,
    ...authorization,
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
  const [mfaRequireConfigStepUp, setMfaRequireConfigStepUp] = useState<
    boolean | null
  >(null);
  const [totpRequireJellyfinLogin, setTotpRequireJellyfinLogin] = useState(false);
  const initialized = useRef(false);

  useEffect(() => {
    if (initialized.current) return;
    initialized.current = true;

    let cancelled = false;
    void loadAuthBootstrapSnapshot().then((snapshot) => {
      if (cancelled) {
        return;
      }
      setToken(snapshot.token);
      setUser(snapshot.user);
      setEffectiveFormLoginEnabled(snapshot.effectiveFormLoginEnabled);
      setPasskeyEnabled(snapshot.passkeyEnabled);
      setMfaRequirePasswordLogin(snapshot.mfaRequirePasswordLogin);
      setMfaRequireConfigStepUp(snapshot.mfaRequireConfigStepUp);
      setTotpRequireJellyfinLogin(snapshot.totpRequireJellyfinLogin);
      setLoading(false);
    });

    return () => {
      cancelled = true;
      initialized.current = false;
    };
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
    const nextUser = normalizeAuthUser(data.login.user) ?? userFromToken(newToken);

    if (options?.persistSession !== false) {
      applyAuthenticatedSession(newToken, nextUser, setToken, setUser);
      rememberAuthBootstrapSession(newToken, nextUser);
    }

    return {
      token: newToken,
      user: nextUser,
      mfaEnrollmentRequired: data.login.mfaEnrollmentRequired === true,
    };
  }, []);

  const adoptSession = useCallback((nextToken: string, nextUser: AuthUser | null) => {
    const normalizedUser = normalizeAuthUser(nextUser) ?? userFromToken(nextToken);
    applyAuthenticatedSession(nextToken, normalizedUser, setToken, setUser);
    rememberAuthBootstrapSession(nextToken, normalizedUser);
  }, []);

  const logout = useCallback(() => {
    clearPersistedAuthToken();
    setToken(null);
    setUser(null);
    rememberAuthBootstrapSession(null, null);
  }, []);

  return {
    token,
    user,
    loading,
    effectiveFormLoginEnabled,
    passkeyEnabled,
    mfaRequirePasswordLogin,
    mfaRequireConfigStepUp,
    totpRequireJellyfinLogin,
    login,
    adoptSession,
    logout,
  };
}
