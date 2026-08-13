import { useCallback, useEffect, useState } from "react";
import { useNavigate, useSearchParams } from "react-router";
import { Fingerprint, KeyRound, Loader2 } from "lucide-react";
import { TotpQrCode } from "@/components/common/totp-qr-code";
import { useAuth, type AuthUser } from "@/lib/hooks/use-auth";
import { useLanguage } from "@/lib/hooks/use-language";
import { TotpCodeForm, sanitizeTotpCode } from "@/components/auth/totp-code-form";
import { Input, integerInputProps } from "@/components/ui/input";
import { useBackendRestarting } from "@/lib/hooks/use-backend-restarting";
import { BackendRestartOverlay } from "@/components/common/backend-restart-overlay";
import { isVisibleExternalAccountProvider } from "@/lib/constants/integration-providers";
import { backendClient, mfaEnrollmentClient } from "@/lib/graphql/urql-client";
import { externalAuthRuntimeSettingsQuery } from "@/lib/graphql/queries";
import {
  completeLoginMfaEnrollmentMutation,
  loginWithEmbyMutation,
  loginWithJellyfinMutation,
  loginWithPlexMutation,
  totpEnrollmentStartMutation,
} from "@/lib/graphql/mutations";
import type {
  ExternalAuthRuntimeSettings,
  TotpEnrollmentComplete,
  TotpEnrollmentStart,
} from "@/lib/types/settings";
import { authenticateWithPasskey, PasskeyClientError } from "@/lib/utils/passkeys";
import { authenticateWithPlexPin } from "@/lib/utils/plex-oauth";
import { selectorId } from "@/lib/utils/dom-ids";

type LoginMethod = "password" | "jellyfin" | "emby" | null;

type LoginPayload = {
  token: string;
  user: AuthUser | null;
  mfaEnrollmentRequired: boolean;
  mfaVerifiedUntil: string | null;
  persistSession: boolean;
};

function resolveRedirectTarget(value: string | null): string {
  if (!value || !value.startsWith("/") || value.startsWith("//")) {
    return "/";
  }

  return value;
}

function connectionOptionLabel(connection: { displayName: string }): string {
  return connection.displayName;
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

function primaryLoginFailureMessage(
  t: (key: string) => string,
  error?: unknown,
): string {
  if (error !== undefined && graphQlErrorCode(error) === "RATE_LIMITED") {
    return t("auth.signInRateLimited");
  }
  return t("auth.signInFailedGeneric");
}

const AUTH_PAGE_CLASS =
  "flex min-h-screen items-center justify-center bg-fixed p-4 text-[var(--scry-body)] [background-image:var(--scry-shell-bg)] sm:p-6";
const AUTH_PANEL_CLASS =
  "w-full max-w-sm space-y-5 rounded-[12px] border border-[var(--scry-border2)] bg-[linear-gradient(180deg,var(--scry-soft),var(--scry-bg))] p-7 shadow-[0_22px_70px_rgba(2,6,23,0.26)] max-sm:p-5";
const AUTH_MFA_PANEL_CLASS =
  "w-full max-w-md space-y-5 rounded-[12px] border border-[var(--scry-border2)] bg-[linear-gradient(180deg,var(--scry-soft),var(--scry-bg))] p-7 shadow-[0_22px_70px_rgba(2,6,23,0.26)] max-sm:p-5";
const AUTH_HEADING_CLASS =
  "text-center font-[var(--font-space-grotesk)] text-2xl font-semibold tracking-normal text-[var(--scry-ink)]";
const AUTH_MUTED_TEXT_CLASS = "text-sm leading-6 text-[var(--scry-muted)]";
const AUTH_LABEL_CLASS = "block text-sm font-medium text-[var(--scry-muted)]";
const AUTH_INPUT_CLASS =
  "h-10 rounded-[9px] border-[var(--scry-border3)] bg-[var(--scry-inset)] text-[var(--scry-ink2)] placeholder:text-[var(--scry-muted3)] focus-visible:border-[var(--scry-accent-ring)] focus-visible:ring-[rgba(var(--scry-accent-rgb),0.25)]";
const AUTH_SELECT_CLASS =
  "h-10 w-full rounded-[9px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] px-3 text-sm text-[var(--scry-ink2)] outline-none focus:border-[var(--scry-accent-ring)] focus:ring-2 focus:ring-[rgba(var(--scry-accent-rgb),0.25)]";
const AUTH_PRIMARY_BUTTON_CLASS =
  "flex h-10 w-full items-center justify-center gap-2 rounded-[9px] bg-primary px-4 text-sm font-semibold text-primary-foreground shadow-none transition-colors hover:bg-primary/90 disabled:opacity-50";
const AUTH_SECONDARY_BUTTON_CLASS =
  "flex h-10 w-full items-center justify-center gap-2 rounded-[9px] border border-[var(--scry-border2)] bg-[var(--scry-inset)] px-4 text-sm font-semibold text-[var(--scry-ink2)] shadow-none transition-colors hover:bg-[var(--scry-hover)] disabled:opacity-50";
const AUTH_ERROR_CLASS =
  "rounded-[9px] border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] px-3 py-2 text-sm leading-6 text-[var(--scry-danger-text)]";

export default function LoginPage() {
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const { serviceRestarting } = useBackendRestarting();
  const { t } = useLanguage(searchParams);
  const {
    login,
    adoptSession,
    logout,
    user,
    loading: authLoading,
    effectiveFormLoginEnabled,
    passkeyEnabled,
  } = useAuth();
  // Default to the Scryer password method so its form is in the DOM and visible
  // at first paint. Password managers detect credential fields on load; a form
  // that only mounts after a chooser click is never offered for autofill. This
  // also pins the form across the async settings load below, which would
  // otherwise raise the method count and unmount an already-detected form.
  const [activeMethod, setActiveMethod] = useState<LoginMethod>("password");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [localTotpCode, setLocalTotpCode] = useState("");
  const [localTotpPrompted, setLocalTotpPrompted] = useState(false);
  const [passkeySubmitting, setPasskeySubmitting] = useState(false);
  const [jellyfinSubmitting, setJellyfinSubmitting] = useState(false);
  const [embySubmitting, setEmbySubmitting] = useState(false);
  const [externalAuthSettings, setExternalAuthSettings] =
    useState<ExternalAuthRuntimeSettings | null>(null);
  const [jellyfinConnectionId, setJellyfinConnectionId] = useState("");
  const [embyConnectionId, setEmbyConnectionId] = useState("");
  const [embyMode, setEmbyMode] = useState<"LOCAL" | "CONNECT">("LOCAL");
  const [embyUsername, setEmbyUsername] = useState("");
  const [embyPassword, setEmbyPassword] = useState("");
  const [embyTotpCode, setEmbyTotpCode] = useState("");
  const [embyTotpPrompted, setEmbyTotpPrompted] = useState(false);
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
  const jellyfinConnections =
    externalAuthSettings?.loginProviders.includes("JELLYFIN")
      ? externalAuthSettings.connections.filter(
          (connection) => connection.provider === "JELLYFIN" && connection.loginEnabled,
        )
      : [];
  const embyConnections =
    externalAuthSettings?.loginProviders.includes("EMBY")
      ? externalAuthSettings.connections.filter(
          (connection) => connection.provider === "EMBY" && connection.loginEnabled,
        )
      : [];
  const plexConnections =
    isVisibleExternalAccountProvider("PLEX") &&
    externalAuthSettings?.loginProviders.includes("PLEX")
      ? externalAuthSettings.connections.filter(
          (connection) => connection.provider === "PLEX" && connection.loginEnabled,
        )
      : [];
  const plexLoginAvailable = plexConnections.length > 0;
  const localPasswordAvailable = effectiveFormLoginEnabled !== false;
  const jellyfinLoginAvailable = jellyfinConnections.length > 0;
  const embyLoginAvailable = embyConnections.length > 0;
  const loginMethodCount = [
    localPasswordAvailable,
    passkeyEnabled,
    jellyfinLoginAvailable,
    embyLoginAvailable,
    plexLoginAvailable,
  ].filter(Boolean).length;
  const showLoginMethodChooser = loginMethodCount > 1;
  const passwordFormVisible =
    activeMethod === "password" ||
    (!showLoginMethodChooser && localPasswordAvailable);
  const jellyfinFormVisible =
    activeMethod === "jellyfin" ||
    (!showLoginMethodChooser && jellyfinLoginAvailable);
  const embyFormVisible =
    activeMethod === "emby" || (!showLoginMethodChooser && embyLoginAvailable);
  const selectedEmbyConnection = embyConnections.find(
    (connection) => connection.id === embyConnectionId,
  );
  const showJellyfinTotpCode = jellyfinTotpPrompted;
  const anySubmitting =
    submitting ||
    passkeySubmitting ||
    jellyfinSubmitting ||
    embySubmitting ||
    jellyfinMfaBusy ||
    plexSubmitting;

  const resetJellyfinTotpChallenge = useCallback(() => {
    setJellyfinTotpPrompted(false);
    setJellyfinTotpCode("");
    setError(null);
  }, []);

  const resetEmbyTotpChallenge = useCallback(() => {
    setEmbyTotpPrompted(false);
    setEmbyTotpCode("");
    setError(null);
  }, []);

  const resetLocalTotpChallenge = useCallback(() => {
    setLocalTotpPrompted(false);
    setLocalTotpCode("");
    setError(null);
  }, []);

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
          .query<{ externalAuthRuntimeSettings?: ExternalAuthRuntimeSettings }>(
            externalAuthRuntimeSettingsQuery,
            {},
            { requestPolicy: "network-only" },
          )
          .toPromise();
        if (error || cancelled) return;
        const settings = data?.externalAuthRuntimeSettings ?? null;
        setExternalAuthSettings(settings);
        const firstJellyfinConnectionId =
          settings?.connections.find(
            (connection) => connection.provider === "JELLYFIN" && connection.loginEnabled,
          )?.id ??
          "";
        if (firstJellyfinConnectionId) {
          setJellyfinConnectionId((current) =>
            current || firstJellyfinConnectionId,
          );
        }
        const firstEmbyConnectionId =
          settings?.connections.find(
            (connection) => connection.provider === "EMBY" && connection.loginEnabled,
          )?.id ?? "";
        if (firstEmbyConnectionId) {
          setEmbyConnectionId((current) => current || firstEmbyConnectionId);
        }
        if (isVisibleExternalAccountProvider("PLEX")) {
          const firstPlexConnectionId =
            settings?.connections.find(
              (connection) => connection.provider === "PLEX" && connection.loginEnabled,
            )?.id ??
            "";
          if (firstPlexConnectionId) {
            setPlexConnectionId((current) => current || firstPlexConnectionId);
          }
        }
      } catch {
        // Provider login remains hidden when settings cannot be loaded.
      }
    })();

    return () => {
      cancelled = true;
    };
  }, []);

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
            setError(primaryLoginFailureMessage(t, err));
          }
          return;
        }

        setError(primaryLoginFailureMessage(t, err));
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
      const { data, error } = await mfaEnrollmentClient
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

  const handleSubmit = useCallback(
    async (e?: React.FormEvent) => {
      e?.preventDefault();
      setError(null);
      setSubmitting(true);
      try {
        const result = await login(username, password, {
          totpCode: localTotpPrompted ? localTotpCode || null : null,
        });
        if (result.mfaEnrollmentRequired) {
          setJellyfinMfaSetupActive(true);
          adoptSession(result.token, result.user ?? null);
          await startJellyfinMfaEnrollment();
          return;
        }
        navigate(redirectTarget, { replace: true });
      } catch (err) {
        if (graphQlErrorCode(err) === "MFA_STEP_UP_REQUIRED") {
          setLocalTotpPrompted(true);
          setLocalTotpCode("");
          setError(null);
        } else {
          setError(primaryLoginFailureMessage(t, err));
        }
      } finally {
        setSubmitting(false);
      }
    },
    [
      adoptSession,
      localTotpCode,
      localTotpPrompted,
      login,
      navigate,
      password,
      redirectTarget,
      startJellyfinMfaEnrollment,
      t,
      username,
    ],
  );

  const completeJellyfinMfaEnrollment = useCallback(async () => {
    if (!jellyfinMfaEnrollment || jellyfinMfaEnrollmentCode.length !== 6) return;

    setError(null);
    setJellyfinMfaBusy(true);
    try {
      const { data, error } = await mfaEnrollmentClient
        .mutation<
          {
            completeLoginMfaEnrollment?: TotpEnrollmentComplete & {
              login: LoginPayload;
            };
          },
          { input: { challengeId: string; code: string } }
        >(completeLoginMfaEnrollmentMutation, {
          input: {
            challengeId: jellyfinMfaEnrollment.challengeId,
            code: jellyfinMfaEnrollmentCode,
          },
        })
        .toPromise();
      if (error || !data?.completeLoginMfaEnrollment) {
        throw error ?? new Error(t("auth.mfaSetupCompleteFailed"));
      }
      setJellyfinMfaRecoveryCodes(data.completeLoginMfaEnrollment.recoveryCodes);
      setJellyfinMfaEnrollment(null);
      setJellyfinMfaEnrollmentCode("");
      adoptSession(
        data.completeLoginMfaEnrollment.login.token,
        data.completeLoginMfaEnrollment.login.user ?? null,
      );
    } catch (err) {
      setError(err instanceof Error ? err.message : t("auth.mfaSetupCompleteFailed"));
    } finally {
      setJellyfinMfaBusy(false);
    }
  }, [adoptSession, jellyfinMfaEnrollment, jellyfinMfaEnrollmentCode, t]);

  const continueAfterJellyfinMfaEnrollment = useCallback(() => {
    setJellyfinMfaSetupActive(false);
    navigate(redirectTarget, { replace: true });
  }, [navigate, redirectTarget]);

  const cancelJellyfinMfaEnrollment = useCallback(() => {
    logout();
    setJellyfinMfaSetupActive(false);
    setJellyfinMfaEnrollment(null);
    setJellyfinMfaEnrollmentCode("");
    setJellyfinMfaRecoveryCodes([]);
    setLocalTotpCode("");
    setLocalTotpPrompted(false);
    setJellyfinPassword("");
    setJellyfinTotpCode("");
    setJellyfinTotpPrompted(false);
    setError(null);
  }, [logout]);

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
              totpCode: jellyfinTotpPrompted ? jellyfinTotpCode || null : null,
              persistSession: true,
            },
          })
          .toPromise();
        if (error || !data?.loginWithJellyfin) {
          throw error ?? new Error(t("auth.jellyfinFailed"));
        }
        const loginPayload = data.loginWithJellyfin;
        if (loginPayload.mfaEnrollmentRequired) {
          setJellyfinMfaSetupActive(true);
          adoptSession(
            loginPayload.token,
            loginPayload.user ?? null,
            loginPayload.persistSession,
          );
          await startJellyfinMfaEnrollment();
          return;
        }
        adoptSession(
          loginPayload.token,
          loginPayload.user ?? null,
          loginPayload.persistSession,
        );
        navigate(redirectTarget, { replace: true });
      } catch (err) {
        if (graphQlErrorCode(err) === "MFA_STEP_UP_REQUIRED") {
          setJellyfinTotpPrompted(true);
          setJellyfinTotpCode("");
          setError(null);
        } else {
          setError(primaryLoginFailureMessage(t));
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
      jellyfinTotpPrompted,
      jellyfinUsername,
      navigate,
      redirectTarget,
      startJellyfinMfaEnrollment,
      t,
    ],
  );

  const handleEmbySignIn = useCallback(async () => {
    if (!embyConnectionId || !embyUsername || !embyPassword) return;

    setError(null);
    setEmbySubmitting(true);
    try {
      const { data, error } = await backendClient
        .mutation(loginWithEmbyMutation, {
          input: {
            connectionId: embyConnectionId,
            mode: embyMode,
            username: embyUsername,
            password: embyPassword,
            totpCode: embyTotpPrompted ? embyTotpCode || null : null,
            persistSession: true,
          },
        })
        .toPromise();
      if (error || !data?.loginWithEmby) {
        throw error ?? new Error("Emby sign-in failed");
      }
      const loginPayload = data.loginWithEmby;
      if (loginPayload.mfaEnrollmentRequired) {
        setJellyfinMfaSetupActive(true);
        adoptSession(
          loginPayload.token,
          loginPayload.user ?? null,
          loginPayload.persistSession,
        );
        await startJellyfinMfaEnrollment();
        return;
      }
      adoptSession(
        loginPayload.token,
        loginPayload.user ?? null,
        loginPayload.persistSession,
      );
      navigate(redirectTarget, { replace: true });
    } catch (err) {
      if (graphQlErrorCode(err) === "MFA_STEP_UP_REQUIRED") {
        setEmbyTotpPrompted(true);
        setEmbyTotpCode("");
        setError(null);
      } else {
        setError("Emby sign-in failed");
      }
    } finally {
      setEmbySubmitting(false);
    }
  }, [
    adoptSession,
    embyConnectionId,
    embyMode,
    embyPassword,
    embyTotpCode,
    embyTotpPrompted,
    embyUsername,
    navigate,
    redirectTarget,
    startJellyfinMfaEnrollment,
  ]);

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
        adoptSession(
          data.loginWithPlex.token,
          data.loginWithPlex.user ?? null,
          data.loginWithPlex.persistSession,
        );
        navigate(redirectTarget, { replace: true });
      } catch (err) {
        setError(primaryLoginFailureMessage(t, err));
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
      <div className={AUTH_PAGE_CLASS}>
        <Loader2 className="h-6 w-6 animate-spin text-[var(--scry-accent-ring)]" />
      </div>
    );
  }

  if (jellyfinMfaSetupActive) {
    return (
      <div className={AUTH_PAGE_CLASS}>
        <div className={AUTH_MFA_PANEL_CLASS}>
          <div className="space-y-2 text-center">
            <h1 className={AUTH_HEADING_CLASS}>{t("auth.mfaSetupTitle")}</h1>
            <p className={AUTH_MUTED_TEXT_CLASS}>{t("auth.mfaSetupDescription")}</p>
          </div>

          {error ? (
            <div id="login-error" className={AUTH_ERROR_CLASS}>
              {error}
            </div>
          ) : null}

          {jellyfinMfaRecoveryCodes.length > 0 ? (
            <div className="space-y-4">
              <p className={AUTH_MUTED_TEXT_CLASS}>
                {t("auth.mfaRecoveryCodesDescription")}
              </p>
              <div className="grid grid-cols-2 gap-2 rounded-[9px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] p-3 font-[var(--font-code)] text-xs text-[var(--scry-ink2)]">
                {jellyfinMfaRecoveryCodes.map((code) => (
                  <code key={code}>{code}</code>
                ))}
              </div>
              <button
                id="jellyfin-mfa-enrollment-continue"
                type="button"
                onClick={continueAfterJellyfinMfaEnrollment}
                className={AUTH_PRIMARY_BUTTON_CLASS}
              >
                {t("auth.continue")}
              </button>
            </div>
          ) : jellyfinMfaEnrollment ? (
            <div className="space-y-4">
              <div className="flex flex-col items-center gap-4">
                <TotpQrCode
                  id="jellyfin-mfa-enrollment-qr-code"
                  value={jellyfinMfaEnrollment.otpauthUrl}
                />
                <a
                  id="jellyfin-mfa-enrollment-setup-link"
                  className="break-all text-sm font-medium text-[var(--scry-accent-text)] underline-offset-4 hover:underline"
                  href={jellyfinMfaEnrollment.otpauthUrl}
                >
                  {t("profile.totpOpenSetupLink")}
                </a>
                <div className="w-full space-y-1">
                  <div className="text-xs text-[var(--scry-muted)]">{t("profile.totpSecret")}</div>
                  <code
                    id="jellyfin-mfa-enrollment-secret"
                    className="block break-all rounded-[7px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] px-2 py-1 font-[var(--font-code)] text-xs text-[var(--scry-ink2)]"
                  >
                    {jellyfinMfaEnrollment.secretBase32}
                  </code>
                </div>
              </div>
              <div className="space-y-2">
                <Input
                  {...integerInputProps}
                  id="jellyfin-mfa-enrollment-code"
                  autoComplete="one-time-code"
                  maxLength={6}
                  value={jellyfinMfaEnrollmentCode}
                  onChange={(event) => setJellyfinMfaEnrollmentCode(sanitizeTotpCode(event.target.value))}
                  placeholder={t("auth.totpCode")}
                  className={AUTH_INPUT_CLASS}
                />
                <button
                  id="jellyfin-mfa-enrollment-submit"
                  type="button"
                  onClick={completeJellyfinMfaEnrollment}
                  disabled={jellyfinMfaBusy || jellyfinMfaEnrollmentCode.length !== 6}
                  className={AUTH_PRIMARY_BUTTON_CLASS}
                >
                  {jellyfinMfaBusy ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                  {t("profile.totpVerifyAndEnable")}
                </button>
              </div>
              <button
                type="button"
                onClick={cancelJellyfinMfaEnrollment}
                disabled={jellyfinMfaBusy}
                className={AUTH_SECONDARY_BUTTON_CLASS}
              >
                {t("auth.mfaSetupCancel")}
              </button>
            </div>
          ) : (
            <div className="space-y-3">
              <div className="flex justify-center">
                <Loader2 className="h-5 w-5 animate-spin text-[var(--scry-muted)]" />
              </div>
              <button
                type="button"
                onClick={startJellyfinMfaEnrollment}
                disabled={jellyfinMfaBusy}
                className={AUTH_SECONDARY_BUTTON_CLASS}
              >
                {t("auth.mfaSetupRestart")}
              </button>
              <button
                type="button"
                onClick={cancelJellyfinMfaEnrollment}
                disabled={jellyfinMfaBusy}
                className={AUTH_SECONDARY_BUTTON_CLASS}
              >
                {t("auth.mfaSetupCancel")}
              </button>
            </div>
          )}
        </div>
      </div>
    );
  }

  return (
    <div className={AUTH_PAGE_CLASS}>
      <div className={AUTH_PANEL_CLASS}>
        <h1 className={AUTH_HEADING_CLASS}>{t("auth.signIn")}</h1>

        {error && (
          <div id="login-error" className={AUTH_ERROR_CLASS}>
            {error}
          </div>
        )}

        <div className="space-y-3">
          {localPasswordAvailable ? (
            <>
              {showLoginMethodChooser ? (
                <button
                  id="login-password-method"
                  type="button"
                  onClick={() =>
                    setActiveMethod((current) =>
                      current === "password" ? null : "password",
                    )
                  }
                  disabled={anySubmitting}
                  aria-controls="login-form"
                  aria-expanded={activeMethod === "password"}
                  className={AUTH_SECONDARY_BUTTON_CLASS}
                >
                  <KeyRound className="h-4 w-4" aria-hidden="true" />
                  {t("auth.signInWithScryerPassword")}
                </button>
              ) : null}

              {passwordFormVisible ? (
                localTotpPrompted ? (
                  <TotpCodeForm
                    id="login-form"
                    inputId="local-totp-code"
                    submitId="login-submit"
                    code={localTotpCode}
                    title={t("auth.totpCode")}
                    description={t("auth.totpCodeRequired")}
                    submitLabel={t("auth.signIn")}
                    busyLabel={t("auth.signingIn")}
                    cancelLabel={t("label.back")}
                    busy={submitting}
                    disabled={anySubmitting && !submitting}
                    onCodeChange={setLocalTotpCode}
                    onSubmit={() => handleSubmit()}
                    onCancel={resetLocalTotpChallenge}
                  />
                ) : (
                  <form id="login-form" onSubmit={handleSubmit} className="space-y-5">
                    <div className="space-y-1.5">
                      <label
                        htmlFor="username"
                        className={AUTH_LABEL_CLASS}
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
                        onChange={(e) => {
                          setUsername(e.target.value);
                          resetLocalTotpChallenge();
                        }}
                        placeholder={t("auth.username")}
                        className={AUTH_INPUT_CLASS}
                      />
                    </div>

                    <div className="space-y-1.5">
                      <label
                        htmlFor="password"
                        className={AUTH_LABEL_CLASS}
                      >
                        {t("auth.password")}
                      </label>
                      <Input
                        id="password"
                        type="password"
                        autoComplete="current-password"
                        required
                        value={password}
                        onChange={(e) => {
                          setPassword(e.target.value);
                          resetLocalTotpChallenge();
                        }}
                        placeholder={t("auth.password")}
                        className={AUTH_INPUT_CLASS}
                      />
                    </div>

                    <button
                      id="login-submit"
                      type="submit"
                      disabled={anySubmitting}
                      className={AUTH_PRIMARY_BUTTON_CLASS}
                    >
                      {submitting ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                      {submitting ? t("auth.signingIn") : t("auth.signIn")}
                    </button>
                  </form>
                )
              ) : null}
            </>
          ) : null}

          {passkeyEnabled ? (
            <button
              id="login-passkey-submit"
              type="button"
              onClick={handlePasskeySignIn}
              disabled={anySubmitting}
              className={AUTH_SECONDARY_BUTTON_CLASS}
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
              {showLoginMethodChooser ? (
                <button
                  id="login-jellyfin-method"
                  type="button"
                  onClick={() =>
                    setActiveMethod((current) =>
                      current === "jellyfin" ? null : "jellyfin",
                    )
                  }
                  disabled={anySubmitting}
                  aria-controls="jellyfin-login-form"
                  aria-expanded={activeMethod === "jellyfin"}
                  className={AUTH_SECONDARY_BUTTON_CLASS}
                >
                  <img
                    src="/auth-providers/jellyfin.svg"
                    alt=""
                    aria-hidden="true"
                    className="h-4 w-4"
                  />
                  {t("auth.signInWithJellyfin")}
                </button>
              ) : null}

              {jellyfinFormVisible && showJellyfinTotpCode ? (
                <TotpCodeForm
                  id="jellyfin-login-form"
                  inputId="jellyfin-totp-code"
                  submitId="jellyfin-login-totp-submit"
                  code={jellyfinTotpCode}
                  title={t("auth.totpCode")}
                  description={t("auth.totpCodeRequired")}
                  submitLabel={t("auth.signIn")}
                  busyLabel={t("auth.signingIn")}
                  cancelLabel={t("label.back")}
                  busy={jellyfinSubmitting}
                  disabled={
                    (anySubmitting && !jellyfinSubmitting) ||
                    !jellyfinConnectionId ||
                    !jellyfinUsername ||
                    !jellyfinPassword
                  }
                  onCodeChange={setJellyfinTotpCode}
                  onSubmit={handleJellyfinSignIn}
                  onCancel={resetJellyfinTotpChallenge}
                />
              ) : jellyfinFormVisible ? (
                // Jellyfin credentials are a separate account from the Scryer
                // login, so this form opts out of password-manager autofill and
                // save prompts. It is still a real form so Enter submits.
                <form
                  id="jellyfin-login-form"
                  className="space-y-3"
                  onSubmit={(event) => {
                    event.preventDefault();
                    void handleJellyfinSignIn();
                  }}
                >
                  {jellyfinConnections.length > 1 ? (
                    <select
                      id="login-jellyfin-connection"
                      className={AUTH_SELECT_CLASS}
                      value={jellyfinConnectionId}
                      onChange={(event) => {
                        setJellyfinConnectionId(event.target.value);
                        resetJellyfinTotpChallenge();
                      }}
                    >
                      {jellyfinConnections.map((connection) => (
                        <option
                          id={selectorId(
                            "login-jellyfin-connection-option",
                            connection.id,
                          )}
                          key={connection.id}
                          value={connection.id}
                        >
                          {connectionOptionLabel(connection)}
                        </option>
                      ))}
                    </select>
                  ) : null}
                  <Input
                    id="jellyfin-username"
                    type="text"
                    ignorePasswordManagers
                    value={jellyfinUsername}
                    onChange={(event) => {
                      setJellyfinUsername(event.target.value);
                      resetJellyfinTotpChallenge();
                    }}
                    placeholder={t("auth.username")}
                    className={AUTH_INPUT_CLASS}
                  />
                  <Input
                    id="jellyfin-password"
                    type="password"
                    ignorePasswordManagers
                    value={jellyfinPassword}
                    onChange={(event) => {
                      setJellyfinPassword(event.target.value);
                      resetJellyfinTotpChallenge();
                    }}
                    placeholder={t("auth.password")}
                    className={AUTH_INPUT_CLASS}
                  />
                  <button
                    id="jellyfin-login-submit"
                    type="submit"
                    disabled={
                      anySubmitting ||
                      !jellyfinConnectionId ||
                      !jellyfinUsername ||
                      !jellyfinPassword ||
                      (jellyfinTotpPrompted && jellyfinTotpCode.length !== 6)
                    }
                    className={AUTH_PRIMARY_BUTTON_CLASS}
                  >
                    {jellyfinSubmitting ? (
                      <Loader2 className="h-4 w-4 animate-spin" />
                    ) : null}
                    {jellyfinSubmitting ? t("auth.signingIn") : t("auth.signIn")}
                  </button>
                </form>
              ) : null}
            </>
          ) : null}

          {embyLoginAvailable ? (
            <>
              {showLoginMethodChooser ? (
                <button
                  id="login-emby-method"
                  type="button"
                  onClick={() =>
                    setActiveMethod((current) => (current === "emby" ? null : "emby"))
                  }
                  disabled={anySubmitting}
                  aria-controls="emby-login-form"
                  aria-expanded={activeMethod === "emby"}
                  className={AUTH_SECONDARY_BUTTON_CLASS}
                >
                  <img
                    src="/auth-providers/emby.svg"
                    alt=""
                    aria-hidden="true"
                    className="h-4 w-4"
                  />
                  Sign in with Emby
                </button>
              ) : null}

              {embyFormVisible && embyTotpPrompted ? (
                <TotpCodeForm
                  id="emby-login-form"
                  inputId="emby-totp-code"
                  submitId="login-emby-submit"
                  code={embyTotpCode}
                  title={t("auth.totpCode")}
                  description={t("auth.totpCodeRequired")}
                  submitLabel={t("auth.signIn")}
                  busyLabel={t("auth.signingIn")}
                  cancelLabel={t("label.back")}
                  busy={embySubmitting}
                  disabled={anySubmitting && !embySubmitting}
                  onCodeChange={setEmbyTotpCode}
                  onSubmit={handleEmbySignIn}
                  onCancel={resetEmbyTotpChallenge}
                />
              ) : embyFormVisible ? (
                <form
                  id="emby-login-form"
                  className="space-y-3"
                  onSubmit={(event) => {
                    event.preventDefault();
                    void handleEmbySignIn();
                  }}
                >
                  <select
                    id="login-emby-connection"
                    className={AUTH_SELECT_CLASS}
                    value={embyConnectionId}
                    onChange={(event) => {
                      const nextId = event.target.value;
                      const nextConnection = embyConnections.find(
                        (connection) => connection.id === nextId,
                      );
                      setEmbyConnectionId(nextId);
                      if (!nextConnection?.embyConnectEnabled) setEmbyMode("LOCAL");
                      resetEmbyTotpChallenge();
                    }}
                  >
                    {embyConnections.map((connection) => (
                      <option
                        id={selectorId("login-emby-connection-option", connection.id)}
                        key={connection.id}
                        value={connection.id}
                      >
                        {connectionOptionLabel(connection)}
                      </option>
                    ))}
                  </select>
                  {selectedEmbyConnection?.embyConnectEnabled ? (
                    <div className="grid grid-cols-2 gap-2">
                      <button
                        id="login-emby-mode-local"
                        type="button"
                        aria-pressed={embyMode === "LOCAL"}
                        onClick={() => setEmbyMode("LOCAL")}
                        className={AUTH_SECONDARY_BUTTON_CLASS}
                      >
                        Local
                      </button>
                      <button
                        id="login-emby-mode-connect"
                        type="button"
                        aria-pressed={embyMode === "CONNECT"}
                        onClick={() => setEmbyMode("CONNECT")}
                        className={AUTH_SECONDARY_BUTTON_CLASS}
                      >
                        Connect
                      </button>
                    </div>
                  ) : null}
                  <Input
                    id="login-emby-username"
                    type="text"
                    ignorePasswordManagers
                    value={embyUsername}
                    onChange={(event) => {
                      setEmbyUsername(event.target.value);
                      resetEmbyTotpChallenge();
                    }}
                    placeholder={
                      embyMode === "CONNECT" ? "Emby Connect username or email" : t("auth.username")
                    }
                    className={AUTH_INPUT_CLASS}
                  />
                  <Input
                    id="login-emby-password"
                    type="password"
                    ignorePasswordManagers
                    value={embyPassword}
                    onChange={(event) => {
                      setEmbyPassword(event.target.value);
                      resetEmbyTotpChallenge();
                    }}
                    placeholder={t("auth.password")}
                    className={AUTH_INPUT_CLASS}
                  />
                  <button
                    id="login-emby-submit"
                    type="submit"
                    disabled={anySubmitting || !embyConnectionId || !embyUsername || !embyPassword}
                    className={AUTH_PRIMARY_BUTTON_CLASS}
                  >
                    {embySubmitting ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                    {embySubmitting ? t("auth.signingIn") : t("auth.signIn")}
                  </button>
                </form>
              ) : null}
            </>
          ) : null}

          {plexLoginAvailable ? (
            <div className="space-y-3">
              {plexConnections.length > 1 ? (
                <select
                  id="login-plex-connection"
                  className={AUTH_SELECT_CLASS}
                  value={plexConnectionId}
                  onChange={(event) => setPlexConnectionId(event.target.value)}
                >
                  {plexConnections.map((connection) => (
                    <option
                      id={selectorId("login-plex-connection-option", connection.id)}
                      key={connection.id}
                      value={connection.id}
                    >
                      {connectionOptionLabel(connection)}
                    </option>
                  ))}
                </select>
              ) : null}
              <button
                id="login-plex-submit"
                type="button"
                onClick={handlePlexSignIn}
                disabled={anySubmitting || !plexConnectionId}
                className={AUTH_SECONDARY_BUTTON_CLASS}
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
