import { useCallback, useEffect, useState } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";
import { Fingerprint, KeyRound, Loader2 } from "lucide-react";
import { QRCode } from "react-qr-code";
import { useAuth } from "@/lib/hooks/use-auth";
import { useLanguage } from "@/lib/hooks/use-language";
import { Input } from "@/components/ui/input";
import { useBackendRestarting } from "@/lib/hooks/use-backend-restarting";
import { BackendRestartOverlay } from "@/components/common/backend-restart-overlay";
import { backendClient } from "@/lib/graphql/urql-client";
import { authProviderRuntimeSettingsQuery } from "@/lib/graphql/queries";
import {
  loginWithJellyfinMutation,
  loginWithPlexMutation,
  totpEnrollmentCompleteMutation,
  totpEnrollmentStartMutation,
} from "@/lib/graphql/mutations";
import type {
  AuthProviderSettings,
  TotpEnrollmentComplete,
  TotpEnrollmentStart,
} from "@/lib/types/settings";
import { authenticateWithPasskey, PasskeyClientError } from "@/lib/utils/passkeys";
import { authenticateWithPlexPin } from "@/lib/utils/plex-oauth";

type LoginMethod = "password" | "jellyfin" | null;

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

function graphQlErrorCode(error: unknown): string | null {
  if (
    error &&
    typeof error === "object" &&
    "graphQLErrors" in error &&
    Array.isArray((error as { graphQLErrors?: unknown[] }).graphQLErrors)
  ) {
    const graphQLErrors = (error as {
      graphQLErrors?: Array<{ extensions?: { code?: unknown } }>;
    }).graphQLErrors;
    const code = graphQLErrors?.find(
      (entry) => typeof entry.extensions?.code === "string",
    )?.extensions?.code;
    return typeof code === "string" ? code : null;
  }

  return null;
}

export default function LoginPage() {
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const { serviceRestarting } = useBackendRestarting();
  const { t } = useLanguage(searchParams);
  const {
    login,
    adoptSession,
    user,
    loading: authLoading,
    effectiveFormLoginEnabled,
    passkeyEnabled,
    totpRequireJellyfinLogin,
  } = useAuth();
  const [activeMethod, setActiveMethod] = useState<LoginMethod>(null);
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
  const [jellyfinTotpCode, setJellyfinTotpCode] = useState("");
  const [jellyfinTotpPrompted, setJellyfinTotpPrompted] = useState(false);
  const [jellyfinMfaSetupActive, setJellyfinMfaSetupActive] = useState(false);
  const [jellyfinMfaEnrollment, setJellyfinMfaEnrollment] =
    useState<TotpEnrollmentStart | null>(null);
  const [jellyfinMfaEnrollmentCode, setJellyfinMfaEnrollmentCode] = useState("");
  const [jellyfinMfaRecoveryCodes, setJellyfinMfaRecoveryCodes] = useState<string[]>([]);
  const [jellyfinMfaBusy, setJellyfinMfaBusy] = useState(false);
  const [plexSubmitting, setPlexSubmitting] = useState(false);
  const redirectTarget = resolveRedirectTarget(searchParams.get("redirect"));
  const availableJellyfinConnections =
    authProviderSettings?.allowedJellyfinConnections?.length
      ? authProviderSettings.allowedJellyfinConnections
      : (authProviderSettings?.allowedJellyfinConnectionIds ?? []).map((id) => ({
          id,
          displayName: "Jellyfin",
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
  const localPasswordAvailable = effectiveFormLoginEnabled !== false;
  const jellyfinLoginAvailable = jellyfinConnections.length > 0;
  const showJellyfinTotpCode = totpRequireJellyfinLogin || jellyfinTotpPrompted;
  const anySubmitting =
    submitting ||
    passkeySubmitting ||
    jellyfinSubmitting ||
    jellyfinMfaBusy ||
    plexSubmitting;

  // Redirect to home if already authenticated
  useEffect(() => {
    if (!serviceRestarting && !authLoading && user && !jellyfinMfaSetupActive) {
      navigate(redirectTarget, { replace: true });
    }
  }, [
    authLoading,
    jellyfinMfaSetupActive,
    user,
    navigate,
    redirectTarget,
    serviceRestarting,
  ]);

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

  const startJellyfinMfaEnrollment = useCallback(async () => {
    setJellyfinMfaBusy(true);
    setJellyfinMfaRecoveryCodes([]);
    setJellyfinMfaEnrollmentCode("");
    try {
      const { data, error } = await backendClient
        .mutation<{ totpEnrollmentStart?: TotpEnrollmentStart }>(
          totpEnrollmentStartMutation,
          {},
        )
        .toPromise();
      if (error || !data?.totpEnrollmentStart) {
        throw error ?? new Error(t("auth.mfaSetupStartFailed"));
      }
      setJellyfinMfaEnrollment(data.totpEnrollmentStart);
    } catch (err) {
      setError(err instanceof Error ? err.message : t("auth.mfaSetupStartFailed"));
    } finally {
      setJellyfinMfaBusy(false);
    }
  }, [t]);

  const completeJellyfinMfaEnrollment = useCallback(async () => {
    if (!jellyfinMfaEnrollment || !jellyfinMfaEnrollmentCode.trim()) return;

    setError(null);
    setJellyfinMfaBusy(true);
    try {
      const { data, error } = await backendClient
        .mutation<
          { totpEnrollmentComplete?: TotpEnrollmentComplete },
          { input: { challengeId: string; code: string } }
        >(totpEnrollmentCompleteMutation, {
          input: {
            challengeId: jellyfinMfaEnrollment.challengeId,
            code: jellyfinMfaEnrollmentCode.trim(),
          },
        })
        .toPromise();
      if (error || !data?.totpEnrollmentComplete) {
        throw error ?? new Error(t("auth.mfaSetupCompleteFailed"));
      }
      setJellyfinMfaRecoveryCodes(data.totpEnrollmentComplete.recoveryCodes);
      setJellyfinMfaEnrollment(null);
      setJellyfinMfaEnrollmentCode("");
    } catch (err) {
      setError(err instanceof Error ? err.message : t("auth.mfaSetupCompleteFailed"));
    } finally {
      setJellyfinMfaBusy(false);
    }
  }, [jellyfinMfaEnrollment, jellyfinMfaEnrollmentCode, t]);

  const skipJellyfinMfaEnrollment = useCallback(() => {
    setJellyfinMfaSetupActive(false);
    setJellyfinMfaEnrollment(null);
    setJellyfinMfaEnrollmentCode("");
    setJellyfinMfaRecoveryCodes([]);
    navigate(redirectTarget, { replace: true });
  }, [navigate, redirectTarget]);

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
              totpCode: jellyfinTotpCode.trim() || null,
              persistSession: true,
            },
          })
          .toPromise();
        if (error || !data?.loginWithJellyfin) {
          throw error ?? new Error(t("auth.jellyfinFailed"));
        }
        const loginPayload = data.loginWithJellyfin;
        if (totpRequireJellyfinLogin && !loginPayload.mfaVerifiedUntil) {
          setJellyfinMfaSetupActive(true);
          adoptSession(loginPayload.token, loginPayload.user ?? null);
          await startJellyfinMfaEnrollment();
          return;
        }
        adoptSession(loginPayload.token, loginPayload.user ?? null);
        navigate(redirectTarget, { replace: true });
      } catch (err) {
        if (graphQlErrorCode(err) === "TOTP_STEP_UP_REQUIRED") {
          setJellyfinTotpPrompted(true);
          setError(t("auth.totpCodeRequired"));
        } else {
          setError(err instanceof Error ? err.message : t("auth.jellyfinFailed"));
        }
      } finally {
        setJellyfinSubmitting(false);
      }
    },
    [
      adoptSession,
      jellyfinConnectionId,
      jellyfinPassword,
      jellyfinTotpCode,
      jellyfinUsername,
      navigate,
      redirectTarget,
      startJellyfinMfaEnrollment,
      t,
      totpRequireJellyfinLogin,
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

  if (jellyfinMfaSetupActive) {
    return (
      <div className="flex min-h-screen items-center justify-center bg-background p-4 text-foreground">
        <div className="w-full max-w-md space-y-5 rounded-lg border border-border bg-card/70 p-8">
          <div className="space-y-2 text-center">
            <h1 className="text-xl font-semibold tracking-tight">{t("auth.mfaSetupTitle")}</h1>
            <p className="text-sm text-muted-foreground">{t("auth.mfaSetupDescription")}</p>
          </div>

          {error ? (
            <div className="rounded-md bg-red-900/40 px-3 py-2 text-sm text-red-300">
              {error}
            </div>
          ) : null}

          {jellyfinMfaRecoveryCodes.length > 0 ? (
            <div className="space-y-4">
              <p className="text-sm text-muted-foreground">
                {t("auth.mfaRecoveryCodesDescription")}
              </p>
              <div className="grid grid-cols-2 gap-2 rounded-md border border-border bg-background/60 p-3 font-mono text-xs">
                {jellyfinMfaRecoveryCodes.map((code) => (
                  <code key={code}>{code}</code>
                ))}
              </div>
              <button
                type="button"
                onClick={skipJellyfinMfaEnrollment}
                className="flex w-full items-center justify-center gap-2 rounded-md bg-emerald-600 px-4 py-2 text-sm font-medium text-foreground hover:bg-emerald-500"
              >
                {t("auth.continue")}
              </button>
            </div>
          ) : jellyfinMfaEnrollment ? (
            <div className="space-y-4">
              <div className="flex flex-col items-center gap-4">
                <div className="w-fit rounded-md bg-white p-3">
                  <QRCode value={jellyfinMfaEnrollment.otpauthUrl} size={168} />
                </div>
                <a
                  className="break-all text-sm font-medium text-primary underline-offset-4 hover:underline"
                  href={jellyfinMfaEnrollment.otpauthUrl}
                >
                  {t("profile.totpOpenSetupLink")}
                </a>
                <div className="w-full space-y-1">
                  <div className="text-xs text-muted-foreground">{t("profile.totpSecret")}</div>
                  <code className="block break-all rounded bg-background/70 px-2 py-1 font-mono text-xs">
                    {jellyfinMfaEnrollment.secretBase32}
                  </code>
                </div>
              </div>
              <div className="space-y-2">
                <Input
                  id="jellyfin-mfa-enrollment-code"
                  inputMode="numeric"
                  autoComplete="one-time-code"
                  value={jellyfinMfaEnrollmentCode}
                  onChange={(event) => setJellyfinMfaEnrollmentCode(event.target.value)}
                  placeholder={t("auth.totpCode")}
                />
                <button
                  type="button"
                  onClick={completeJellyfinMfaEnrollment}
                  disabled={jellyfinMfaBusy || !jellyfinMfaEnrollmentCode.trim()}
                  className="flex w-full items-center justify-center gap-2 rounded-md bg-emerald-600 px-4 py-2 text-sm font-medium text-foreground hover:bg-emerald-500 disabled:opacity-50"
                >
                  {jellyfinMfaBusy ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                  {t("profile.totpVerifyAndEnable")}
                </button>
              </div>
              <button
                type="button"
                onClick={skipJellyfinMfaEnrollment}
                disabled={jellyfinMfaBusy}
                className="w-full rounded-md border border-border bg-background px-4 py-2 text-sm font-medium text-foreground hover:bg-muted disabled:opacity-50"
              >
                {t("auth.mfaSetupSkip")}
              </button>
            </div>
          ) : (
            <div className="space-y-3">
              <div className="flex justify-center">
                <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
              </div>
              <button
                type="button"
                onClick={startJellyfinMfaEnrollment}
                disabled={jellyfinMfaBusy}
                className="w-full rounded-md border border-border bg-background px-4 py-2 text-sm font-medium text-foreground hover:bg-muted disabled:opacity-50"
              >
                {t("auth.mfaSetupRestart")}
              </button>
              <button
                type="button"
                onClick={skipJellyfinMfaEnrollment}
                disabled={jellyfinMfaBusy}
                className="w-full rounded-md border border-border bg-background px-4 py-2 text-sm font-medium text-foreground hover:bg-muted disabled:opacity-50"
              >
                {t("auth.mfaSetupSkip")}
              </button>
            </div>
          )}
        </div>
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

        <div className="space-y-3">
          {localPasswordAvailable ? (
            <>
              <button
                type="button"
                onClick={() =>
                  setActiveMethod((current) =>
                    current === "password" ? null : "password",
                  )
                }
                disabled={anySubmitting}
                aria-controls="login-form"
                aria-expanded={activeMethod === "password"}
                className="flex w-full items-center justify-center gap-2 rounded-md border border-border bg-background px-4 py-2 text-sm font-medium text-foreground hover:bg-muted disabled:opacity-50"
              >
                <KeyRound className="h-4 w-4" aria-hidden="true" />
                {t("auth.signInWithScryerPassword")}
              </button>

              {activeMethod === "password" ? (
                <form id="login-form" onSubmit={handleSubmit} className="space-y-5">
                  <div className="space-y-1.5">
                    <label
                      htmlFor="username"
                      className="block text-sm font-medium text-muted-foreground"
                    >
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
                    <label
                      htmlFor="password"
                      className="block text-sm font-medium text-muted-foreground"
                    >
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
                    disabled={anySubmitting}
                    className="flex w-full items-center justify-center gap-2 rounded-md bg-emerald-600 px-4 py-2 text-sm font-medium text-foreground hover:bg-emerald-500 disabled:opacity-50"
                  >
                    {submitting ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                    {submitting ? t("auth.signingIn") : t("auth.signIn")}
                  </button>
                </form>
              ) : null}
            </>
          ) : null}

          {passkeyEnabled ? (
            <button
              type="button"
              onClick={handlePasskeySignIn}
              disabled={anySubmitting}
              className="flex w-full items-center justify-center gap-2 rounded-md border border-border bg-background px-4 py-2 text-sm font-medium text-foreground hover:bg-muted disabled:opacity-50"
            >
              {passkeySubmitting ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <Fingerprint className="h-4 w-4" aria-hidden="true" />
              )}
              {passkeySubmitting ? t("auth.passkeySigningIn") : t("auth.signInWithPasskey")}
            </button>
          ) : null}

          {jellyfinLoginAvailable ? (
            <>
              <button
                type="button"
                onClick={() =>
                  setActiveMethod((current) =>
                    current === "jellyfin" ? null : "jellyfin",
                  )
                }
                disabled={anySubmitting}
                aria-controls="jellyfin-login-form"
                aria-expanded={activeMethod === "jellyfin"}
                className="flex w-full items-center justify-center gap-2 rounded-md border border-border bg-background px-4 py-2 text-sm font-medium text-foreground hover:bg-muted disabled:opacity-50"
              >
                <img
                  src="/auth-providers/jellyfin.svg"
                  alt=""
                  aria-hidden="true"
                  className="h-4 w-4"
                />
                {t("auth.signInWithJellyfin")}
              </button>

              {activeMethod === "jellyfin" ? (
                <div id="jellyfin-login-form" className="space-y-3">
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
                  <Input
                    id="jellyfin-username"
                    type="text"
                    autoComplete="username"
                    value={jellyfinUsername}
                    onChange={(event) => setJellyfinUsername(event.target.value)}
                    placeholder={t("auth.username")}
                  />
                  <Input
                    id="jellyfin-password"
                    type="password"
                    autoComplete="current-password"
                    value={jellyfinPassword}
                    onChange={(event) => setJellyfinPassword(event.target.value)}
                    placeholder={t("auth.password")}
                  />
                  {showJellyfinTotpCode ? (
                    <Input
                      id="jellyfin-totp-code"
                      inputMode="numeric"
                      autoComplete="one-time-code"
                      value={jellyfinTotpCode}
                      onChange={(event) => setJellyfinTotpCode(event.target.value)}
                      placeholder={t("auth.totpCode")}
                    />
                  ) : null}
                  <button
                    type="button"
                    onClick={handleJellyfinSignIn}
                    disabled={
                      anySubmitting ||
                      !jellyfinConnectionId ||
                      !jellyfinUsername ||
                      !jellyfinPassword
                    }
                    className="flex w-full items-center justify-center gap-2 rounded-md bg-emerald-600 px-4 py-2 text-sm font-medium text-foreground hover:bg-emerald-500 disabled:opacity-50"
                  >
                    {jellyfinSubmitting ? (
                      <Loader2 className="h-4 w-4 animate-spin" />
                    ) : null}
                    {jellyfinSubmitting ? t("auth.signingIn") : t("auth.signIn")}
                  </button>
                </div>
              ) : null}
            </>
          ) : null}

          {plexLoginAvailable ? (
            <div className="space-y-3">
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
              <button
                type="button"
                onClick={handlePlexSignIn}
                disabled={anySubmitting || !plexConnectionId}
                className="flex w-full items-center justify-center gap-2 rounded-md border border-border bg-background px-4 py-2 text-sm font-medium text-foreground hover:bg-muted disabled:opacity-50"
                title={plexSubmitting ? t("auth.plexPinFlowPending") : undefined}
              >
                {plexSubmitting ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <img
                    src="/auth-providers/plex.svg"
                    alt=""
                    aria-hidden="true"
                    className="h-4 w-4"
                  />
                )}
                {plexSubmitting ? t("auth.plexPinFlowPending") : t("auth.signInWithPlex")}
              </button>
            </div>
          ) : null}
        </div>
      </div>
    </div>
  );
}
