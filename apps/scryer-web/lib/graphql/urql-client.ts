import {
  Client,
  fetchExchange,
  subscriptionExchange,
} from "@urql/core";
import { clearClientAuthSession, getAuthToken } from "@/lib/hooks/use-auth";
import { getRuntimeBasePath, getRuntimeGraphqlUrl } from "@/lib/runtime-config";
import { wsClient } from "@/lib/graphql/ws-client";

// ---------------------------------------------------------------------------
// Shared language ref — updated by the Provider when uiLanguage changes
// ---------------------------------------------------------------------------

let currentLanguage = "eng";

export function setGraphqlLanguage(lang: string) {
  currentLanguage = lang;
}

// ---------------------------------------------------------------------------
// Backend restart detection — when the backend returns HTML (upgrade splash)
// instead of JSON, trigger a global callback so the shell can show the splash
// overlay immediately, regardless of which component made the request.
// ---------------------------------------------------------------------------

let onBackendRestarting: (() => void) | null = null;

export function setOnBackendRestarting(cb: (() => void) | null) {
  onBackendRestarting = cb;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value != null && typeof value === "object";
}

function isAuthenticationRequiredResponse(body: unknown): boolean {
  if (!isRecord(body) || !Array.isArray(body.errors)) {
    return false;
  }

  return body.errors.some((error) => {
    if (!isRecord(error) || typeof error.message !== "string") {
      return false;
    }

    return error.message.trim().toLowerCase() === "authentication required";
  });
}

function getLoginRedirectTarget() {
  if (typeof window === "undefined") {
    return null;
  }

  const basePath = getRuntimeBasePath();
  const pathname = window.location.pathname;

  if (basePath !== "/" && pathname.startsWith(basePath)) {
    return `${pathname.slice(basePath.length) || "/"}${window.location.search}${window.location.hash}`;
  }

  return `${pathname}${window.location.search}${window.location.hash}`;
}

function redirectToLogin() {
  if (typeof window === "undefined") {
    return;
  }

  const basePath = getRuntimeBasePath();
  const redirectTarget = getLoginRedirectTarget();
  const currentAppPath = redirectTarget ?? "/";
  clearClientAuthSession();

  if (currentAppPath === "/login" || currentAppPath.startsWith("/login?")) {
    return;
  }

  const loginPath = basePath === "/" ? "/login" : `${basePath}/login`;
  const params = new URLSearchParams();
  if (redirectTarget && redirectTarget.startsWith("/") && !redirectTarget.startsWith("//")) {
    params.set("redirect", redirectTarget);
  }

  const destination = params.size > 0 ? `${loginPath}?${params.toString()}` : loginPath;
  window.location.replace(destination);
}

export const scryerFetch: typeof fetch = async (input, init) => {
  const response = await fetch(input, init);
  const ct = response.headers.get("content-type") ?? "";
  if (ct.includes("text/html")) {
    onBackendRestarting?.();
    throw new TypeError("Service is restarting");
  }

  if (ct.includes("application/json")) {
    const body = await response.clone().json().catch(() => null);
    if (isAuthenticationRequiredResponse(body)) {
      redirectToLogin();
      throw new TypeError("Authentication required");
    }
  }

  return response;
};

function errorHasName(error: unknown, name: string): boolean {
  return (
    error != null &&
    typeof error === "object" &&
    "name" in error &&
    (error as { name?: unknown }).name === name
  );
}

export function makeAbortableFetch(signal: AbortSignal): typeof fetch {
  return (input, init) => scryerFetch(input, { ...init, signal });
}

export function isAbortError(error: unknown): boolean {
  if (errorHasName(error, "AbortError")) {
    return true;
  }
  if (
    error != null &&
    typeof error === "object" &&
    "networkError" in error
  ) {
    return errorHasName(
      (error as { networkError?: unknown }).networkError,
      "AbortError",
    );
  }
  return false;
}

// ---------------------------------------------------------------------------
// Backend client — connects to the Rust GraphQL server at /graphql
// ---------------------------------------------------------------------------

export const backendClient = new Client({
  url: getRuntimeGraphqlUrl(),
  preferGetMethod: false,
  requestPolicy: "network-only",
  fetch: scryerFetch,
  exchanges: [
    subscriptionExchange({
      forwardSubscription(request) {
        const input = { ...request, query: request.query || "" };
        return {
          subscribe(sink) {
            const unsubscribe = wsClient.subscribe(input, sink);
            return { unsubscribe };
          },
        };
      },
    }),
    fetchExchange,
  ],
  fetchOptions: () => {
    const headers: Record<string, string> = {
      "x-scryer-language": currentLanguage,
    };
    const token = getAuthToken();
    if (token) {
      headers["authorization"] = `Bearer ${token}`;
    }
    return { headers };
  },
});
