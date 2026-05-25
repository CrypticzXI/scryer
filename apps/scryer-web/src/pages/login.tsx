import { useCallback, useEffect, useState } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";
import { Loader2 } from "lucide-react";
import { useAuth } from "@/lib/hooks/use-auth";
import { useLanguage } from "@/lib/hooks/use-language";
import { Input } from "@/components/ui/input";
import { useBackendRestarting } from "@/lib/hooks/use-backend-restarting";
import { BackendRestartOverlay } from "@/components/common/backend-restart-overlay";
import { backendClient } from "@/lib/graphql/urql-client";
import { authProviderRuntimeSettingsQuery } from "@/lib/graphql/queries";
import { loginWithJellyfinMutation, loginWithPlexMutation } from "@/lib/graphql/mutations";
import type { AuthProviderSettings } from "@/lib/types/settings";
import { authenticateWithPasskey, PasskeyClientError } from "@/lib/utils/passkeys";
import { authenticateWithPlexPin } from "@/lib/utils/plex-oauth";

function resolveRedirectTarget(value: string | null): string {
  if (!value || !value.startsWith("/") || value.startsWith("//")) {
    return "/";
  }

  return value;
}

function connectionOptionLabel(connection: {
  displayName: string;
  userVisibleUrl: string | null;
}): string {
  return connection.userVisibleUrl
    ? `${connection.displayName} (${connection.userVisibleUrl})`
    : connection.displayName;
}

export default function LoginPage() {
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const { serviceRestarting } = useBackendRestarting();
  const { t } = useLanguage(searchParams);
  const { login, adoptSession, user, loading: authLoading, passkeyEnabled } = useAuth();
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [passkeySubmitting, setPasskeySubmitting] = useState(false);
  const [jellyfinSubmitting, setJellyfinSubmitting] = useState(false);
  const [authProviderSettings, setAuthProviderSettings] =
    useState<AuthProviderSettings | null>(null);
  const [jellyfinConnectionId, setJellyfinConnectionId] = useState("");
  const [plexConnectionId, setPlexConnectionId] = useState("");
  const [jellyfinUsername, setJellyfinUsername] = useState("");
  const [jellyfinPassword, setJellyfinPassword] = useState("");
  const [plexSubmitting, setPlexSubmitting] = useState(false);
  const redirectTarget = resolveRedirectTarget(searchParams.get("redirect"));
  const availableJellyfinConnections =
    authProviderSettings?.allowedJellyfinConnections?.length
      ? authProviderSettings.allowedJellyfinConnections
      : (authProviderSettings?.allowedJellyfinConnectionIds ?? []).map((id) => ({
          id,
          displayName: id,
          userVisibleUrl: null,
          baseUrl: null,
          machineId: null,
        }));
  const jellyfinConnections =
    authProviderSettings?.providerLoginEnabled.includes("jellyfin") &&
    authProviderSettings.allowedProviders.includes("jellyfin")
      ? availableJellyfinConnections
      : [];
  const availablePlexConnections = authProviderSettings?.allowedPlexConnections?.length
    ? authProviderSettings.allowedPlexConnections
    : (authProviderSettings?.allowedPlexConnectionIds ?? []).map((id) => ({
        id,
        displayName: id,
        userVisibleUrl: null,
        baseUrl: null,
        machineId: null,
      }));
  const plexConnections =
    authProviderSettings?.providerLoginEnabled.includes("plex") &&
    authProviderSettings.allowedProviders.includes("plex")
      ? availablePlexConnections
      : [];
  const plexLoginAvailable = plexConnections.length > 0;

  // Redirect to home if already authenticated
  useEffect(() => {
    if (!serviceRestarting && !authLoading && user) {
      navigate(redirectTarget, { replace: true });
    }
  }, [authLoading, user, navigate, redirectTarget, serviceRestarting]);

  useEffect(() => {
    let cancelled = false;

    (async () => {
      try {
        const { data, error } = await backendClient
          .query<{ authProviderRuntimeSettings?: AuthProviderSettings }>(
            authProviderRuntimeSettingsQuery,
            {},
          )
          .toPromise();
        if (error || cancelled) return;
        const settings = data?.authProviderRuntimeSettings ?? null;
        setAuthProviderSettings(settings);
        const firstJellyfinConnectionId =
          settings?.allowedJellyfinConnections[0]?.id ??
          settings?.allowedJellyfinConnectionIds[0] ??
          "";
        if (firstJellyfinConnectionId) {
          setJellyfinConnectionId((current) =>
            current || firstJellyfinConnectionId,
          );
        }
        const firstPlexConnectionId =
          settings?.allowedPlexConnections[0]?.id ??
          settings?.allowedPlexConnectionIds[0] ??
          "";
        if (firstPlexConnectionId) {
          setPlexConnectionId((current) => current || firstPlexConnectionId);
        }
      } catch {
        // Provider login remains hidden when settings cannot be loaded.
      }
    })();

    return () => {
      cancelled = true;
    };
  }, []);

  const handleSubmit = useCallback(
    async (e: React.FormEvent) => {
      e.preventDefault();
      setError(null);
      setSubmitting(true);
      try {
        await login(username, password);
        navigate(redirectTarget, { replace: true });
      } catch (err) {
        setError(err instanceof Error ? err.message : t("auth.invalidCredentials"));
      } finally {
        setSubmitting(false);
      }
    },
    [login, navigate, password, redirectTarget, t, username],
  );

  const handlePasskeySignIn = useCallback(
    async () => {
      setError(null);
      setPasskeySubmitting(true);
      try {
        const result = await authenticateWithPasskey();
        adoptSession(result.token, result.user);
        navigate(redirectTarget, { replace: true });
      } catch (err) {
        if (err instanceof PasskeyClientError) {
          if (err.code === "unsupported") {
            setError(t("auth.passkeyUnsupported"));
          } else if (err.code === "cancelled") {
            setError(t("auth.passkeyCancelled"));
          } else {
            setError(err.message || t("auth.passkeyFailed"));
          }
          return;
        }

        setError(err instanceof Error ? err.message : t("auth.passkeyFailed"));
      } finally {
        setPasskeySubmitting(false);
      }
    },
    [adoptSession, navigate, redirectTarget, t],
  );

  const handleJellyfinSignIn = useCallback(
    async () => {
      if (!jellyfinConnectionId || !jellyfinUsername || !jellyfinPassword) return;

      setError(null);
      setJellyfinSubmitting(true);
      try {
        const { data, error } = await backendClient
          .mutation(loginWithJellyfinMutation, {
            input: {
              connectionId: jellyfinConnectionId,
              username: jellyfinUsername,
              password: jellyfinPassword,
              persistSession: true,
            },
          })
          .toPromise();
        if (error || !data?.loginWithJellyfin) {
          throw error ?? new Error(t("auth.jellyfinFailed"));
        }
        adoptSession(data.loginWithJellyfin.token, data.loginWithJellyfin.user ?? null);
        navigate(redirectTarget, { replace: true });
      } catch (err) {
        setError(err instanceof Error ? err.message : t("auth.jellyfinFailed"));
      } finally {
        setJellyfinSubmitting(false);
      }
    },
    [
      adoptSession,
      jellyfinConnectionId,
      jellyfinPassword,
      jellyfinUsername,
      navigate,
      redirectTarget,
      t,
    ],
  );

  const handlePlexSignIn = useCallback(
    async () => {
      if (!plexConnectionId) return;

      setError(null);
      setPlexSubmitting(true);
      try {
        const plexAuthToken = await authenticateWithPlexPin();
        const { data, error } = await backendClient
          .mutation(loginWithPlexMutation, {
            input: {
              connectionId: plexConnectionId,
              plexAuthToken,
              persistSession: true,
            },
          })
          .toPromise();
        if (error || !data?.loginWithPlex) {
          throw error ?? new Error(t("auth.plexFailed"));
        }
        adoptSession(data.loginWithPlex.token, data.loginWithPlex.user ?? null);
        navigate(redirectTarget, { replace: true });
      } catch (err) {
        setError(err instanceof Error ? err.message : t("auth.plexFailed"));
      } finally {
        setPlexSubmitting(false);
      }
    },
    [adoptSession, navigate, plexConnectionId, redirectTarget, t],
  );

  if (serviceRestarting) {
    return <BackendRestartOverlay />;
  }

  if (authLoading) {
    return (
      <div className="flex min-h-screen items-center justify-center bg-background text-card-foreground">
        <Loader2 className="h-6 w-6 animate-spin text-emerald-700 dark:text-emerald-300" />
      </div>
    );
  }

  return (
    <div className="flex min-h-screen items-center justify-center bg-background p-4 text-foreground">
      <div className="w-full max-w-sm space-y-5 rounded-lg border border-border bg-card/70 p-8">
        <h1 className="text-center text-xl font-semibold tracking-tight">{t("auth.signIn")}</h1>

        {error && (
          <div className="rounded-md bg-red-900/40 px-3 py-2 text-sm text-red-300">{error}</div>
        )}

        <form id="login-form" onSubmit={handleSubmit} className="space-y-5">
          <div className="space-y-1.5">
            <label htmlFor="username" className="block text-sm font-medium text-muted-foreground">
              {t("auth.username")}
            </label>
            <Input
              id="username"
              type="text"
              autoComplete="username"
              autoFocus
              required
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              placeholder={t("auth.username")}
            />
          </div>

          <div className="space-y-1.5">
            <label htmlFor="password" className="block text-sm font-medium text-muted-foreground">
              {t("auth.password")}
            </label>
            <Input
              id="password"
              type="password"
              autoComplete="current-password"
              required
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              placeholder={t("auth.password")}
            />
          </div>

          <button
            id="login-submit"
            type="submit"
            disabled={submitting || passkeySubmitting || plexSubmitting}
            className="flex w-full items-center justify-center gap-2 rounded-md bg-emerald-600 px-4 py-2 text-sm font-medium text-foreground hover:bg-emerald-500 disabled:opacity-50"
          >
            {submitting && <Loader2 className="h-4 w-4 animate-spin" />}
            {submitting ? t("auth.signingIn") : t("auth.signIn")}
          </button>
        </form>

        {passkeyEnabled ? (
          <>
            <div className="flex items-center gap-3 text-xs uppercase tracking-[0.2em] text-muted-foreground">
              <div className="h-px flex-1 bg-border" />
              <span>{t("label.or")}</span>
              <div className="h-px flex-1 bg-border" />
            </div>
            <button
              type="button"
              onClick={handlePasskeySignIn}
              disabled={submitting || passkeySubmitting || plexSubmitting}
              className="flex w-full items-center justify-center gap-2 rounded-md border border-border bg-background px-4 py-2 text-sm font-medium text-foreground hover:bg-muted disabled:opacity-50"
            >
              {passkeySubmitting ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
              {passkeySubmitting ? t("auth.passkeySigningIn") : t("auth.signInWithPasskey")}
            </button>
          </>
        ) : null}

        {jellyfinConnections.length > 0 ? (
          <>
            <div className="flex items-center gap-3 text-xs uppercase tracking-[0.2em] text-muted-foreground">
              <div className="h-px flex-1 bg-border" />
              <span>{t("label.or")}</span>
              <div className="h-px flex-1 bg-border" />
            </div>
            <div className="space-y-3 rounded-md border border-border bg-background/60 p-3">
              <div className="text-sm font-medium">{t("auth.signInWithJellyfin")}</div>
              <div className="space-y-3">
                {jellyfinConnections.length > 1 ? (
                  <select
                    className="w-full rounded-md border border-border bg-background px-3 py-2 text-sm"
                    value={jellyfinConnectionId}
                    onChange={(event) => setJellyfinConnectionId(event.target.value)}
                  >
                    {jellyfinConnections.map((connection) => (
                      <option key={connection.id} value={connection.id}>
                        {connectionOptionLabel(connection)}
                      </option>
                    ))}
                  </select>
                ) : null}
                {jellyfinConnections.length === 1 ? (
                  <div className="rounded-md border border-border/70 bg-card/50 px-3 py-2 text-xs text-muted-foreground">
                    <div className="font-medium text-foreground">
                      {jellyfinConnections[0].displayName}
                    </div>
                    {jellyfinConnections[0].userVisibleUrl ? (
                      <div>{jellyfinConnections[0].userVisibleUrl}</div>
                    ) : null}
                  </div>
                ) : null}
                <Input
                  type="text"
                  autoComplete="username"
                  value={jellyfinUsername}
                  onChange={(event) => setJellyfinUsername(event.target.value)}
                  placeholder={t("auth.username")}
                />
                <Input
                  type="password"
                  autoComplete="current-password"
                  value={jellyfinPassword}
                  onChange={(event) => setJellyfinPassword(event.target.value)}
                  placeholder={t("auth.password")}
                />
                <button
                  type="button"
                  onClick={handleJellyfinSignIn}
                  disabled={submitting || passkeySubmitting || jellyfinSubmitting || plexSubmitting}
                  className="flex w-full items-center justify-center gap-2 rounded-md border border-border bg-background px-4 py-2 text-sm font-medium text-foreground hover:bg-muted disabled:opacity-50"
                >
                  {jellyfinSubmitting ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                  {jellyfinSubmitting ? t("auth.signingIn") : t("auth.signInWithJellyfin")}
                </button>
              </div>
            </div>
          </>
        ) : null}

        {plexLoginAvailable ? (
          <>
            <div className="flex items-center gap-3 text-xs uppercase tracking-[0.2em] text-muted-foreground">
              <div className="h-px flex-1 bg-border" />
              <span>{t("label.or")}</span>
              <div className="h-px flex-1 bg-border" />
            </div>
            <div className="space-y-3 rounded-md border border-border bg-background/60 p-3">
              {plexConnections.length > 1 ? (
                <select
                  className="w-full rounded-md border border-border bg-background px-3 py-2 text-sm"
                  value={plexConnectionId}
                  onChange={(event) => setPlexConnectionId(event.target.value)}
                >
                  {plexConnections.map((connection) => (
                    <option key={connection.id} value={connection.id}>
                      {connectionOptionLabel(connection)}
                    </option>
                  ))}
                </select>
              ) : null}
              {plexConnections.length === 1 ? (
                <div className="rounded-md border border-border/70 bg-card/50 px-3 py-2 text-xs text-muted-foreground">
                  <div className="font-medium text-foreground">
                    {plexConnections[0].displayName}
                  </div>
                  {plexConnections[0].userVisibleUrl ? (
                    <div>{plexConnections[0].userVisibleUrl}</div>
                  ) : null}
                </div>
              ) : null}
              <button
                type="button"
                onClick={handlePlexSignIn}
                disabled={submitting || passkeySubmitting || jellyfinSubmitting || plexSubmitting}
                className="flex w-full items-center justify-center gap-2 rounded-md border border-border bg-background px-4 py-2 text-sm font-medium text-foreground hover:bg-muted disabled:opacity-50"
                title={plexSubmitting ? t("auth.plexPinFlowPending") : undefined}
              >
                {plexSubmitting ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                {plexSubmitting ? t("auth.plexPinFlowPending") : t("auth.signInWithPlex")}
              </button>
            </div>
          </>
        ) : null}
      </div>
    </div>
  );
}
