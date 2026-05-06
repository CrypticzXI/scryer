import * as React from "react";
import { useClient } from "urql";
import { toast } from "sonner";
import { SettingsSecuritySection } from "@/components/views/settings/settings-security-section";
import { disposeWsClient } from "@/lib/graphql/ws-client";
import { updateSecuritySettingsMutation } from "@/lib/graphql/mutations";
import { securitySettingsQuery } from "@/lib/graphql/queries";
import { useTranslate } from "@/lib/context/translate-context";
import { useAuth } from "@/lib/hooks/use-auth";
import type { SecuritySettings } from "@/lib/types/settings";
import { APP_PERMISSIONS, hasAppPermission } from "@/lib/utils/permissions";

const DEFAULT_SECURITY_SETTINGS: SecuritySettings = {
  formLoginEnabled: false,
  skipLoginForLocalIps: false,
  effectiveFormLoginEnabled: false,
  envOverrideActive: false,
  envOverrideDescription: null,
};
function errorMessage(error: unknown, fallback: string) {
  return error instanceof Error && error.message.trim().length > 0
    ? error.message
    : fallback;
}

export function SettingsSecurityContainer() {
  const client = useClient();
  const t = useTranslate();
  const { token, login, adoptSession, logout } = useAuth();
  const [settings, setSettings] = React.useState<SecuritySettings>(
    DEFAULT_SECURITY_SETTINGS,
  );
  const [loading, setLoading] = React.useState(true);
  const [enableConfirmOpen, setEnableConfirmOpen] = React.useState(false);
  const [disableConfirmOpen, setDisableConfirmOpen] = React.useState(false);
  const [confirmBusy, setConfirmBusy] = React.useState(false);
  const [confirmUsername, setConfirmUsername] = React.useState("");
  const [confirmPassword, setConfirmPassword] = React.useState("");
  const [confirmError, setConfirmError] = React.useState<string | null>(null);

  React.useEffect(() => {
    let cancelled = false;

    (async () => {
      try {
        const { data, error } = await client.query(securitySettingsQuery, {}).toPromise();
        if (error) throw error;
        if (cancelled) return;
        setSettings({
          ...DEFAULT_SECURITY_SETTINGS,
          ...data?.securitySettings,
        });
      } catch (error) {
        if (!cancelled) {
          setSettings(DEFAULT_SECURITY_SETTINGS);
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
  ) => {
    const { data, error } = await client
      .mutation(updateSecuritySettingsMutation, {
        input: { formLoginEnabled, skipLoginForLocalIps },
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
    t,
    token,
  ]);

  return (
    <SettingsSecuritySection
      settings={settings}
      loading={loading}
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
    />
  );
}
