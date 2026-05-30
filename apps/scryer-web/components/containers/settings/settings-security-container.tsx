import * as React from "react";
import { useClient } from "urql";
import { toast } from "sonner";
import {
  ExternalAccountInvitesContainer,
  notifyExternalAccountInviteSourcesChanged,
} from "@/components/containers/settings/external-account-invites-container";
import { SettingsSecuritySection } from "@/components/views/settings/settings-security-section";
import { disposeWsClient } from "@/lib/graphql/ws-client";
import {
  testJellyfinConnectionMutation,
  updateAuthProviderSettingsMutation,
  updateSecuritySettingsMutation,
} from "@/lib/graphql/mutations";
import { authProviderSettingsQuery, securitySettingsQuery } from "@/lib/graphql/queries";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useAuth } from "@/lib/hooks/use-auth";
import type { AuthProviderSettings, SecuritySettings } from "@/lib/types/settings";
import {
  isReportedConnectionFeedbackError,
  runConnectionFeedback,
} from "@/lib/utils/connection-feedback";
import { APP_PERMISSIONS, hasAppPermission } from "@/lib/utils/permissions";

const DEFAULT_SECURITY_SETTINGS: SecuritySettings = {
  formLoginEnabled: false,
  skipLoginForLocalIps: false,
  totpRequireConfigStepUp: false,
  totpRequireJellyfinLogin: false,
  effectiveFormLoginEnabled: false,
  envOverrideActive: false,
  envOverrideDescription: null,
};

const DEFAULT_AUTH_PROVIDER_SETTINGS: AuthProviderSettings = {
  allowedProviders: [],
  providerLoginEnabled: [],
  providerLinkingEnabled: [],
  allowedJellyfinConnectionIds: [],
  allowedPlexConnectionIds: [],
  allowedJellyfinConnections: [],
  allowedPlexConnections: [],
};

function errorMessage(error: unknown, fallback: string) {
  return error instanceof Error && error.message.trim().length > 0
    ? error.message
    : fallback;
}

export function SettingsSecurityContainer() {
  const client = useClient();
  const t = useTranslate();
  const setGlobalStatus = useGlobalStatus();
  const { token, user, login, adoptSession, logout } = useAuth();
  const [settings, setSettings] = React.useState<SecuritySettings>(
    DEFAULT_SECURITY_SETTINGS,
  );
  const [authProviderSettings, setAuthProviderSettings] =
    React.useState<AuthProviderSettings>(DEFAULT_AUTH_PROVIDER_SETTINGS);
  const [loading, setLoading] = React.useState(true);
  const [authProviderSaving, setAuthProviderSaving] = React.useState(false);
  const [enableConfirmOpen, setEnableConfirmOpen] = React.useState(false);
  const [disableConfirmOpen, setDisableConfirmOpen] = React.useState(false);
  const [confirmBusy, setConfirmBusy] = React.useState(false);
  const [confirmUsername, setConfirmUsername] = React.useState("");
  const [confirmPassword, setConfirmPassword] = React.useState("");
  const [confirmError, setConfirmError] = React.useState<string | null>(null);
  const canManageExternalInvites =
    user != null && hasAppPermission(user, APP_PERMISSIONS.manageUsers);

  React.useEffect(() => {
    let cancelled = false;

    (async () => {
      try {
        const [securityResult, authProviderResult] = await Promise.all([
          client.query(securitySettingsQuery, {}).toPromise(),
          client.query(authProviderSettingsQuery, {}).toPromise(),
        ]);
        if (securityResult.error) throw securityResult.error;
        if (authProviderResult.error) throw authProviderResult.error;
        if (cancelled) return;
        setSettings({
          ...DEFAULT_SECURITY_SETTINGS,
          ...securityResult.data?.securitySettings,
        });
        setAuthProviderSettings({
          ...DEFAULT_AUTH_PROVIDER_SETTINGS,
          ...authProviderResult.data?.authProviderSettings,
        });
      } catch (error) {
        if (!cancelled) {
          setSettings(DEFAULT_SECURITY_SETTINGS);
          setAuthProviderSettings(DEFAULT_AUTH_PROVIDER_SETTINGS);
          toast.error(errorMessage(error, t("settings.securityLoadFailed")));
        }
      } finally {
        if (!cancelled) {
          setLoading(false);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [client, t]);

  const applySecuritySettings = React.useCallback(async (
    formLoginEnabled: boolean,
    skipLoginForLocalIps: boolean,
    totpRequireConfigStepUp: boolean,
    totpRequireJellyfinLogin: boolean,
  ) => {
    const { data, error } = await client
      .mutation(updateSecuritySettingsMutation, {
        input: {
          formLoginEnabled,
          skipLoginForLocalIps,
          totpRequireConfigStepUp,
          totpRequireJellyfinLogin,
        },
      })
      .toPromise();

    if (error || !data?.updateSecuritySettings) {
      throw error ?? new Error(t("settings.securitySaveFailed"));
    }

    return {
      ...DEFAULT_SECURITY_SETTINGS,
      ...data.updateSecuritySettings,
    } as SecuritySettings;
  }, [client, t]);

  const handleToggle = React.useCallback((enabled: boolean) => {
    if (enabled === settings.formLoginEnabled || confirmBusy) {
      return;
    }

    if (enabled) {
      setConfirmError(null);
      setEnableConfirmOpen(true);
      return;
    }

    setDisableConfirmOpen(true);
  }, [
    confirmBusy,
    settings.formLoginEnabled,
  ]);

  const handleConfirmEnable = React.useCallback(async () => {
    const effectiveChangesNow =
      !settings.envOverrideActive && !settings.effectiveFormLoginEnabled;

    setConfirmBusy(true);
    setConfirmError(null);

    let verifiedSession: Awaited<ReturnType<typeof login>>;
    try {
      verifiedSession = await login(confirmUsername, confirmPassword, {
        persistSession: false,
      });
    } catch {
      setConfirmError(t("settings.securityCredentialsInvalid"));
      setConfirmBusy(false);
      return;
    }

    if (!hasAppPermission(verifiedSession.user, APP_PERMISSIONS.manageUsers)) {
      setConfirmError(t("settings.securityCredentialsInsufficient"));
      setConfirmBusy(false);
      return;
    }

    try {
      const nextSettings = await applySecuritySettings(
        true,
        settings.skipLoginForLocalIps,
        settings.totpRequireConfigStepUp,
        settings.totpRequireJellyfinLogin,
      );
      setSettings(nextSettings);
      toast.success(
        effectiveChangesNow
          ? t("settings.securityEnabledSuccess")
          : t("settings.securityPreferenceSaved"),
      );

      if (effectiveChangesNow) {
        adoptSession(verifiedSession.token, verifiedSession.user);
        disposeWsClient();
        window.location.reload();
        return;
      }

      setEnableConfirmOpen(false);
      setConfirmUsername("");
      setConfirmPassword("");
      setConfirmError(null);
    } catch (error) {
      setConfirmError(errorMessage(error, t("settings.securitySaveFailed")));
      toast.error(errorMessage(error, t("settings.securitySaveFailed")));
    } finally {
      setConfirmBusy(false);
    }
  }, [
    adoptSession,
    applySecuritySettings,
    confirmPassword,
    confirmUsername,
    login,
    settings.effectiveFormLoginEnabled,
    settings.envOverrideActive,
    settings.skipLoginForLocalIps,
    settings.totpRequireConfigStepUp,
    settings.totpRequireJellyfinLogin,
    t,
  ]);

  const handleCancelEnable = React.useCallback(() => {
    if (confirmBusy) {
      return;
    }

    setEnableConfirmOpen(false);
    setConfirmUsername("");
    setConfirmPassword("");
    setConfirmError(null);
  }, [confirmBusy]);

  const handleConfirmDisable = React.useCallback(async () => {
    const effectiveChangesNow =
      !settings.envOverrideActive && settings.effectiveFormLoginEnabled;

    setConfirmBusy(true);
    try {
      const nextSettings = await applySecuritySettings(
        false,
        settings.skipLoginForLocalIps,
        settings.totpRequireConfigStepUp,
        settings.totpRequireJellyfinLogin,
      );
      setSettings(nextSettings);
      toast.success(
        effectiveChangesNow
          ? t("settings.securityDisabledSuccess")
          : t("settings.securityPreferenceSaved"),
      );

      if (effectiveChangesNow) {
        logout();
        disposeWsClient();
        window.location.reload();
        return;
      }

      setDisableConfirmOpen(false);
    } catch (error) {
      toast.error(errorMessage(error, t("settings.securitySaveFailed")));
    } finally {
      setConfirmBusy(false);
    }
  }, [
    applySecuritySettings,
    logout,
    settings.effectiveFormLoginEnabled,
    settings.envOverrideActive,
    settings.skipLoginForLocalIps,
    settings.totpRequireConfigStepUp,
    settings.totpRequireJellyfinLogin,
    t,
  ]);

  const handleCancelDisable = React.useCallback(() => {
    if (confirmBusy) {
      return;
    }

    setDisableConfirmOpen(false);
  }, [confirmBusy]);

  const handleSkipLocalIpsChange = React.useCallback(async (enabled: boolean) => {
    if (confirmBusy || enabled === settings.skipLoginForLocalIps) {
      return;
    }

    try {
      const nextSettings = await applySecuritySettings(
        settings.formLoginEnabled,
        enabled,
        settings.totpRequireConfigStepUp,
        settings.totpRequireJellyfinLogin,
      );
      setSettings(nextSettings);
      toast.success(t("settings.securityPreferenceSaved"));

      const bypassWasRevoked =
        settings.effectiveFormLoginEnabled &&
        settings.skipLoginForLocalIps &&
        !nextSettings.skipLoginForLocalIps;
      if (bypassWasRevoked && !token) {
        logout();
        disposeWsClient();
        window.location.reload();
      }
    } catch (error) {
      toast.error(errorMessage(error, t("settings.securitySaveFailed")));
    }
  }, [
    applySecuritySettings,
    confirmBusy,
    logout,
    settings.effectiveFormLoginEnabled,
    settings.formLoginEnabled,
    settings.skipLoginForLocalIps,
    settings.totpRequireConfigStepUp,
    settings.totpRequireJellyfinLogin,
    t,
    token,
  ]);

  const handleTotpConfigStepUpChange = React.useCallback(async (enabled: boolean) => {
    if (confirmBusy || enabled === settings.totpRequireConfigStepUp) {
      return;
    }

    try {
      const nextSettings = await applySecuritySettings(
        settings.formLoginEnabled,
        settings.skipLoginForLocalIps,
        enabled,
        settings.totpRequireJellyfinLogin,
      );
      setSettings(nextSettings);
      toast.success(t("settings.securityPreferenceSaved"));
    } catch (error) {
      toast.error(errorMessage(error, t("settings.securitySaveFailed")));
    }
  }, [
    applySecuritySettings,
    confirmBusy,
    settings.formLoginEnabled,
    settings.skipLoginForLocalIps,
    settings.totpRequireConfigStepUp,
    settings.totpRequireJellyfinLogin,
    t,
  ]);

  const handleTotpJellyfinLoginChange = React.useCallback(async (enabled: boolean) => {
    if (confirmBusy || enabled === settings.totpRequireJellyfinLogin) {
      return;
    }

    try {
      const nextSettings = await applySecuritySettings(
        settings.formLoginEnabled,
        settings.skipLoginForLocalIps,
        settings.totpRequireConfigStepUp,
        enabled,
      );
      setSettings(nextSettings);
      toast.success(t("settings.securityPreferenceSaved"));
    } catch (error) {
      toast.error(errorMessage(error, t("settings.securitySaveFailed")));
    }
  }, [
    applySecuritySettings,
    confirmBusy,
    settings.formLoginEnabled,
    settings.skipLoginForLocalIps,
    settings.totpRequireConfigStepUp,
    settings.totpRequireJellyfinLogin,
    t,
  ]);

  const handleAuthProviderSettingsSave = React.useCallback(async () => {
    setAuthProviderSaving(true);
    try {
      const allowedJellyfinConnections =
        authProviderSettings.allowedJellyfinConnections.map((connection) => ({
          id: connection.id,
          displayName: connection.displayName,
          baseUrl: connection.baseUrl,
          machineId: null,
        }));
      const allowedPlexConnections =
        authProviderSettings.allowedPlexConnections.map((connection) => ({
          id: connection.id,
          displayName: connection.displayName,
          baseUrl: null,
          machineId: connection.machineId,
        }));

      for (const connection of allowedJellyfinConnections) {
        if (!connection.baseUrl?.trim()) {
          continue;
        }

        const connectionLabel =
          connection.displayName.trim() || connection.baseUrl.trim() || "Jellyfin";

        await runConnectionFeedback({
          setGlobalStatus,
          startMessage: t("status.testingJellyfinConnection", {
            connection: connectionLabel,
          }),
          successMessage: t("status.jellyfinConnectionTestPassed", {
            connection: connectionLabel,
          }),
          failureFallbackMessage: t("status.jellyfinConnectionTestFailed", {
            connection: connectionLabel,
          }),
          announceSuccess: false,
          run: async () => {
            const { data: testData, error: testError } = await client
              .mutation(testJellyfinConnectionMutation, {
                input: {
                  connection: {
                    id: connection.id.trim() || null,
                    displayName: connection.displayName,
                    baseUrl: connection.baseUrl,
                    machineId: null,
                  },
                },
              })
              .toPromise();
            if (testError) throw testError;
            if (!testData?.testJellyfinConnection) {
              throw new Error(
                t("status.jellyfinConnectionTestFailed", {
                  connection: connectionLabel,
                }),
              );
            }
          },
        });
      }

      const { data, error } = await client
        .mutation(updateAuthProviderSettingsMutation, {
          input: {
            allowedProviders: authProviderSettings.allowedProviders,
            providerLoginEnabled: authProviderSettings.providerLoginEnabled,
            providerLinkingEnabled: authProviderSettings.providerLinkingEnabled,
            allowedJellyfinConnectionIds: allowedJellyfinConnections
              .map((connection) => connection.id.trim())
              .filter(Boolean),
            allowedPlexConnectionIds: allowedPlexConnections
              .map((connection) => connection.id.trim())
              .filter(Boolean),
            allowedJellyfinConnections,
            allowedPlexConnections,
          },
        })
        .toPromise();
      if (error || !data?.updateAuthProviderSettings) {
        throw error ?? new Error(t("settings.securitySaveFailed"));
      }
      setAuthProviderSettings({
        ...DEFAULT_AUTH_PROVIDER_SETTINGS,
        ...data.updateAuthProviderSettings,
      });
      notifyExternalAccountInviteSourcesChanged();
      setGlobalStatus(t("settings.securityPreferenceSaved"));
      toast.success(t("settings.securityPreferenceSaved"));
    } catch (error) {
      if (!isReportedConnectionFeedbackError(error)) {
        toast.error(errorMessage(error, t("settings.securitySaveFailed")));
      }
    } finally {
      setAuthProviderSaving(false);
    }
  }, [authProviderSettings, client, setGlobalStatus, t]);

  return (
    <SettingsSecuritySection
      settings={settings}
      authProviderSettings={authProviderSettings}
      loading={loading}
      authProviderSaving={authProviderSaving}
      enableConfirmOpen={enableConfirmOpen}
      disableConfirmOpen={disableConfirmOpen}
      confirmBusy={confirmBusy}
      confirmUsername={confirmUsername}
      confirmPassword={confirmPassword}
      confirmError={confirmError}
      onToggle={handleToggle}
      onConfirmUsernameChange={setConfirmUsername}
      onConfirmPasswordChange={setConfirmPassword}
      onConfirmEnable={handleConfirmEnable}
      onCancelEnable={handleCancelEnable}
      onConfirmDisable={handleConfirmDisable}
      onCancelDisable={handleCancelDisable}
      onSkipLocalIpsChange={handleSkipLocalIpsChange}
      onTotpConfigStepUpChange={handleTotpConfigStepUpChange}
      onTotpJellyfinLoginChange={handleTotpJellyfinLoginChange}
      onAuthProviderSettingsChange={setAuthProviderSettings}
      onAuthProviderSettingsSave={handleAuthProviderSettingsSave}
      externalAccountInvitesPanel={
        canManageExternalInvites ? <ExternalAccountInvitesContainer /> : null
      }
    />
  );
}
