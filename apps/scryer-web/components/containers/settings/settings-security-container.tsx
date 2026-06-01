import * as React from "react";
import { useClient } from "urql";
import { toast } from "sonner";
import { ExternalAccountInvitesContainer } from "@/components/containers/settings/external-account-invites-container";
import { SettingsSecuritySection } from "@/components/views/settings/settings-security-section";
import { disposeWsClient } from "@/lib/graphql/ws-client";
import { updateSecuritySettingsMutation } from "@/lib/graphql/mutations";
import { securitySettingsQuery } from "@/lib/graphql/queries";
import { useTranslate } from "@/lib/context/translate-context";
import { useAuth } from "@/lib/hooks/use-auth";
import type { SecuritySettings } from "@/lib/types/settings";
import { APP_PERMISSIONS, hasAppPermission } from "@/lib/utils/permissions";

const MIN_PASSWORD_LENGTH = 8;

const DEFAULT_SECURITY_SETTINGS: SecuritySettings = {
  formLoginEnabled: false,
  passwordMinLength: MIN_PASSWORD_LENGTH,
  skipLoginForLocalIps: false,
  totpRequireConfigStepUp: false,
  totpRequireLocalLogin: false,
  totpRequireJellyfinLogin: false,
  effectiveFormLoginEnabled: false,
  envOverrideActive: false,
  envOverrideDescription: null,
};

function errorMessage(error: unknown, fallback: string) {
  return error instanceof Error && error.message.trim().length > 0
    ? error.message
    : fallback;
}

function parsePasswordMinLengthDraft(value: string): number | null {
  const trimmed = value.trim();
  if (!/^\d+$/.test(trimmed)) {
    return null;
  }

  const parsed = Number.parseInt(trimmed, 10);
  if (Number.isNaN(parsed) || parsed < MIN_PASSWORD_LENGTH) {
    return null;
  }

  return parsed;
}

export function SettingsSecurityContainer() {
  const client = useClient();
  const t = useTranslate();
  const { token, user, login, adoptSession, logout } = useAuth();
  const [settings, setSettings] = React.useState<SecuritySettings>(
    DEFAULT_SECURITY_SETTINGS,
  );
  const [loading, setLoading] = React.useState(true);
  const [enableConfirmOpen, setEnableConfirmOpen] = React.useState(false);
  const [disableConfirmOpen, setDisableConfirmOpen] = React.useState(false);
  const [confirmBusy, setConfirmBusy] = React.useState(false);
  const [saveBusy, setSaveBusy] = React.useState(false);
  const [confirmUsername, setConfirmUsername] = React.useState("");
  const [confirmPassword, setConfirmPassword] = React.useState("");
  const [confirmError, setConfirmError] = React.useState<string | null>(null);
  const [passwordMinLengthDraft, setPasswordMinLengthDraft] = React.useState(
    String(DEFAULT_SECURITY_SETTINGS.passwordMinLength),
  );
  const passwordMinLengthSavePromiseRef = React.useRef<Promise<SecuritySettings | null> | null>(
    null,
  );
  const passwordMinLengthSaveToastRequestedRef = React.useRef(false);
  const canManageExternalInvites =
    user != null && hasAppPermission(user, APP_PERMISSIONS.manageUsers);

  React.useEffect(() => {
    setPasswordMinLengthDraft(String(settings.passwordMinLength));
  }, [settings.passwordMinLength]);

  const draftPasswordMinLength = React.useMemo(
    () => parsePasswordMinLengthDraft(passwordMinLengthDraft),
    [passwordMinLengthDraft],
  );
  const effectivePasswordMinLength =
    draftPasswordMinLength ?? settings.passwordMinLength;

  React.useEffect(() => {
    let cancelled = false;

    (async () => {
      try {
        const securityResult = await client.query(securitySettingsQuery, {}).toPromise();
        if (securityResult.error) throw securityResult.error;
        if (cancelled) return;
        setSettings({
          ...DEFAULT_SECURITY_SETTINGS,
          ...securityResult.data?.securitySettings,
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
    passwordMinLength: number,
    skipLoginForLocalIps: boolean,
    totpRequireConfigStepUp: boolean,
    totpRequireLocalLogin: boolean,
    totpRequireJellyfinLogin: boolean,
  ) => {
    const { data, error } = await client
      .mutation(updateSecuritySettingsMutation, {
        input: {
          formLoginEnabled,
          passwordMinLength,
          skipLoginForLocalIps,
          totpRequireConfigStepUp,
          totpRequireLocalLogin,
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

  const submitPasswordMinLength = React.useCallback(async (showSuccessToast: boolean) => {
    if (showSuccessToast) {
      passwordMinLengthSaveToastRequestedRef.current = true;
    }

    if (passwordMinLengthSavePromiseRef.current) {
      return passwordMinLengthSavePromiseRef.current;
    }

    if (loading || confirmBusy || saveBusy) {
      return null;
    }

    if (draftPasswordMinLength == null) {
      setPasswordMinLengthDraft(String(settings.passwordMinLength));
      passwordMinLengthSaveToastRequestedRef.current = false;
      toast.error(
        t("settings.securityPasswordMinLengthInvalid", {
          min: MIN_PASSWORD_LENGTH,
        }),
      );
      return null;
    }

    if (draftPasswordMinLength === settings.passwordMinLength) {
      setPasswordMinLengthDraft(String(draftPasswordMinLength));
      passwordMinLengthSaveToastRequestedRef.current = false;
      return null;
    }

    const savePromise = (async () => {
      setSaveBusy(true);
      try {
        const nextSettings = await applySecuritySettings(
          settings.formLoginEnabled,
          draftPasswordMinLength,
          settings.skipLoginForLocalIps,
          settings.totpRequireConfigStepUp,
          settings.totpRequireLocalLogin,
          settings.totpRequireJellyfinLogin,
        );
        setSettings(nextSettings);
        if (passwordMinLengthSaveToastRequestedRef.current) {
          toast.success(t("settings.securityPreferenceSaved"));
        }
        return nextSettings;
      } catch (error) {
        setPasswordMinLengthDraft(String(settings.passwordMinLength));
        toast.error(errorMessage(error, t("settings.securitySaveFailed")));
        return null;
      } finally {
        passwordMinLengthSaveToastRequestedRef.current = false;
        passwordMinLengthSavePromiseRef.current = null;
        setSaveBusy(false);
      }
    })();

    passwordMinLengthSavePromiseRef.current = savePromise;
    return savePromise;
  }, [
    applySecuritySettings,
    confirmBusy,
    draftPasswordMinLength,
    loading,
    saveBusy,
    settings.formLoginEnabled,
    settings.passwordMinLength,
    settings.skipLoginForLocalIps,
    settings.totpRequireConfigStepUp,
    settings.totpRequireJellyfinLogin,
    settings.totpRequireLocalLogin,
    t,
  ]);

  const handleToggle = React.useCallback((enabled: boolean) => {
    if (enabled === settings.formLoginEnabled || confirmBusy || saveBusy) {
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
    saveBusy,
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
      await submitPasswordMinLength(false);
      const nextSettings = await applySecuritySettings(
        true,
        effectivePasswordMinLength,
        settings.skipLoginForLocalIps,
        settings.totpRequireConfigStepUp,
        settings.totpRequireLocalLogin,
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
    submitPasswordMinLength,
    settings.totpRequireConfigStepUp,
    settings.totpRequireLocalLogin,
    settings.totpRequireJellyfinLogin,
    t,
    effectivePasswordMinLength,
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
      await submitPasswordMinLength(false);
      const nextSettings = await applySecuritySettings(
        false,
        effectivePasswordMinLength,
        settings.skipLoginForLocalIps,
        settings.totpRequireConfigStepUp,
        settings.totpRequireLocalLogin,
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
    effectivePasswordMinLength,
    logout,
    settings.effectiveFormLoginEnabled,
    settings.envOverrideActive,
    settings.skipLoginForLocalIps,
    submitPasswordMinLength,
    settings.totpRequireConfigStepUp,
    settings.totpRequireLocalLogin,
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
    if (confirmBusy || saveBusy || enabled === settings.skipLoginForLocalIps) {
      return;
    }

    await submitPasswordMinLength(false);
    setSaveBusy(true);
    try {
      const nextSettings = await applySecuritySettings(
        settings.formLoginEnabled,
        effectivePasswordMinLength,
        enabled,
        settings.totpRequireConfigStepUp,
        settings.totpRequireLocalLogin,
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
    } finally {
      setSaveBusy(false);
    }
  }, [
    applySecuritySettings,
    confirmBusy,
    effectivePasswordMinLength,
    saveBusy,
    logout,
    settings.effectiveFormLoginEnabled,
    settings.formLoginEnabled,
    settings.skipLoginForLocalIps,
    submitPasswordMinLength,
    settings.totpRequireConfigStepUp,
    settings.totpRequireLocalLogin,
    settings.totpRequireJellyfinLogin,
    t,
    token,
  ]);

  const handleTotpConfigStepUpChange = React.useCallback(async (enabled: boolean) => {
    if (confirmBusy || saveBusy || enabled === settings.totpRequireConfigStepUp) {
      return;
    }

    await submitPasswordMinLength(false);
    setSaveBusy(true);
    try {
      const nextSettings = await applySecuritySettings(
        settings.formLoginEnabled,
        effectivePasswordMinLength,
        settings.skipLoginForLocalIps,
        enabled,
        settings.totpRequireLocalLogin,
        settings.totpRequireJellyfinLogin,
      );
      setSettings(nextSettings);
      toast.success(t("settings.securityPreferenceSaved"));
    } catch (error) {
      toast.error(errorMessage(error, t("settings.securitySaveFailed")));
    } finally {
      setSaveBusy(false);
    }
  }, [
    applySecuritySettings,
    confirmBusy,
    effectivePasswordMinLength,
    saveBusy,
    settings.formLoginEnabled,
    settings.skipLoginForLocalIps,
    submitPasswordMinLength,
    settings.totpRequireConfigStepUp,
    settings.totpRequireLocalLogin,
    settings.totpRequireJellyfinLogin,
    t,
  ]);

  const handleTotpLocalLoginChange = React.useCallback(async (enabled: boolean) => {
    if (confirmBusy || saveBusy || enabled === settings.totpRequireLocalLogin) {
      return;
    }

    await submitPasswordMinLength(false);
    setSaveBusy(true);
    try {
      const nextSettings = await applySecuritySettings(
        settings.formLoginEnabled,
        effectivePasswordMinLength,
        settings.skipLoginForLocalIps,
        settings.totpRequireConfigStepUp,
        enabled,
        settings.totpRequireJellyfinLogin,
      );
      setSettings(nextSettings);
      toast.success(t("settings.securityPreferenceSaved"));
    } catch (error) {
      toast.error(errorMessage(error, t("settings.securitySaveFailed")));
    } finally {
      setSaveBusy(false);
    }
  }, [
    applySecuritySettings,
    confirmBusy,
    effectivePasswordMinLength,
    saveBusy,
    settings.formLoginEnabled,
    settings.skipLoginForLocalIps,
    submitPasswordMinLength,
    settings.totpRequireConfigStepUp,
    settings.totpRequireLocalLogin,
    settings.totpRequireJellyfinLogin,
    t,
  ]);

  const handleTotpJellyfinLoginChange = React.useCallback(async (enabled: boolean) => {
    if (confirmBusy || saveBusy || enabled === settings.totpRequireJellyfinLogin) {
      return;
    }

    await submitPasswordMinLength(false);
    setSaveBusy(true);
    try {
      const nextSettings = await applySecuritySettings(
        settings.formLoginEnabled,
        effectivePasswordMinLength,
        settings.skipLoginForLocalIps,
        settings.totpRequireConfigStepUp,
        settings.totpRequireLocalLogin,
        enabled,
      );
      setSettings(nextSettings);
      toast.success(t("settings.securityPreferenceSaved"));
    } catch (error) {
      toast.error(errorMessage(error, t("settings.securitySaveFailed")));
    } finally {
      setSaveBusy(false);
    }
  }, [
    applySecuritySettings,
    confirmBusy,
    effectivePasswordMinLength,
    saveBusy,
    settings.formLoginEnabled,
    settings.skipLoginForLocalIps,
    submitPasswordMinLength,
    settings.totpRequireConfigStepUp,
    settings.totpRequireLocalLogin,
    settings.totpRequireJellyfinLogin,
    t,
  ]);

  const handlePasswordMinLengthSubmit = React.useCallback(async () => {
    await submitPasswordMinLength(true);
  }, [submitPasswordMinLength]);

  return (
    <SettingsSecuritySection
      settings={settings}
      loading={loading || saveBusy}
      enableConfirmOpen={enableConfirmOpen}
      disableConfirmOpen={disableConfirmOpen}
      confirmBusy={confirmBusy}
      confirmUsername={confirmUsername}
      confirmPassword={confirmPassword}
      confirmError={confirmError}
      passwordMinLengthDraft={passwordMinLengthDraft}
      minPasswordLength={MIN_PASSWORD_LENGTH}
      onToggle={handleToggle}
      onConfirmUsernameChange={setConfirmUsername}
      onConfirmPasswordChange={setConfirmPassword}
      onConfirmEnable={handleConfirmEnable}
      onCancelEnable={handleCancelEnable}
      onConfirmDisable={handleConfirmDisable}
      onCancelDisable={handleCancelDisable}
      onPasswordMinLengthDraftChange={setPasswordMinLengthDraft}
      onPasswordMinLengthSubmit={handlePasswordMinLengthSubmit}
      onSkipLocalIpsChange={handleSkipLocalIpsChange}
      onTotpConfigStepUpChange={handleTotpConfigStepUpChange}
      onTotpLocalLoginChange={handleTotpLocalLoginChange}
      onTotpJellyfinLoginChange={handleTotpJellyfinLoginChange}
      externalAccountInvitesPanel={
        canManageExternalInvites ? <ExternalAccountInvitesContainer /> : null
      }
    />
  );
}
