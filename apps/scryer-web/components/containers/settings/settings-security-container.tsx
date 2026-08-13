import * as React from "react";
import { useNavigate } from "react-router";
import { useClient } from "urql";
import { toast } from "sonner";
import { ExternalAccountInvitesContainer } from "@/components/containers/settings/external-account-invites-container";
import { SettingsSecuritySection } from "@/components/views/settings/settings-security-section";
import { disposeWsClient } from "@/lib/graphql/ws-client";
import { updateSecuritySettingsMutation } from "@/lib/graphql/mutations";
import { securitySettingsQuery } from "@/lib/graphql/queries";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import { useTranslate } from "@/lib/context/translate-context";
import { useAuth } from "@/lib/hooks/use-auth";
import type { SecuritySettings } from "@/lib/types/settings";
import { APP_PERMISSIONS, hasAppPermission } from "@/lib/utils/permissions";
import { LatestWinsSaveQueue } from "@/lib/utils/latest-wins-save-queue";

const MIN_PASSWORD_LENGTH = 8;
const DEFAULT_ADMIN_PASSWORD_FORM_LOGIN_ERROR =
  "change the default admin password before enabling form login";

const DEFAULT_SECURITY_SETTINGS: SecuritySettings = {
  formLoginEnabled: false,
  passwordMinLength: MIN_PASSWORD_LENGTH,
  skipLoginForLocalIps: false,
  mfaRequireConfigStepUp: false,
  mfaRequirePasswordLogin: false,
  totpRequireJellyfinLogin: false,
  totpRequireEmbyLogin: false,
  effectiveFormLoginEnabled: false,
  envOverrideActive: false,
  envOverrideDescription: null,
};

function errorMessage(error: unknown, fallback: string) {
  return userFacingGraphQlErrorMessage(error, fallback);
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
  const navigate = useNavigate();
  const client = useClient();
  const t = useTranslate();
  const { token, user, login, adoptSession, logout } = useAuth();
  const [settings, setSettings] = React.useState<SecuritySettings>(
    DEFAULT_SECURITY_SETTINGS,
  );
  const [loading, setLoading] = React.useState(true);
  const [enableConfirmOpen, setEnableConfirmOpen] = React.useState(false);
  const [disableConfirmOpen, setDisableConfirmOpen] = React.useState(false);
  const [adminPasswordRequiredOpen, setAdminPasswordRequiredOpen] = React.useState(false);
  const [confirmBusy, setConfirmBusy] = React.useState(false);
  const [saveBusy, setSaveBusy] = React.useState(false);
  const [confirmPassword, setConfirmPassword] = React.useState("");
  const [confirmError, setConfirmError] = React.useState<string | null>(null);
  const [passwordMinLengthDraft, setPasswordMinLengthDraft] = React.useState(
    String(DEFAULT_SECURITY_SETTINGS.passwordMinLength),
  );
  const settingsRef = React.useRef(settings);
  const passwordMinLengthSaveQueueRef = React.useRef<LatestWinsSaveQueue<string> | null>(null);
  if (passwordMinLengthSaveQueueRef.current == null) {
    passwordMinLengthSaveQueueRef.current = new LatestWinsSaveQueue<string>();
  }
  const passwordMinLengthSaveToastRequestedRef = React.useRef(false);
  const canManageExternalInvites =
    user != null && hasAppPermission(user, APP_PERMISSIONS.manageUsers);

  React.useEffect(() => {
    settingsRef.current = settings;
  }, [settings]);

  React.useEffect(() => {
    setPasswordMinLengthDraft(String(settings.passwordMinLength));
  }, [settings.passwordMinLength]);

  const draftPasswordMinLength = React.useMemo(
    () => parsePasswordMinLengthDraft(passwordMinLengthDraft),
    [passwordMinLengthDraft],
  );
  const effectivePasswordMinLength =
    draftPasswordMinLength ?? settings.passwordMinLength;

  const openAdminPasswordRequiredDialog = React.useCallback(() => {
    setEnableConfirmOpen(false);
    setConfirmPassword("");
    setConfirmError(null);
    setAdminPasswordRequiredOpen(true);
  }, []);

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
    mfaRequireConfigStepUp: boolean,
    mfaRequirePasswordLogin: boolean,
    totpRequireJellyfinLogin: boolean,
    totpRequireEmbyLogin: boolean = settingsRef.current.totpRequireEmbyLogin,
  ) => {
    const { data, error } = await client
      .mutation(updateSecuritySettingsMutation, {
        input: {
          formLoginEnabled,
          passwordMinLength,
          skipLoginForLocalIps,
          mfaRequireConfigStepUp,
          mfaRequirePasswordLogin,
          totpRequireJellyfinLogin,
          totpRequireEmbyLogin,
        },
      })
      .toPromise();

    if (error || !data?.updateSecuritySettings) {
      throw error ?? new Error(t("settings.securitySaveFailed"));
    }

    const nextSettings = {
      ...DEFAULT_SECURITY_SETTINGS,
      ...data.updateSecuritySettings,
    } as SecuritySettings;
    settingsRef.current = nextSettings;
    return nextSettings;
  }, [client, t]);

  const runPasswordMinLengthSave = React.useCallback(async (submittedDraft: string) => {
    const currentSettings = settingsRef.current;
    const submittedPasswordMinLength = parsePasswordMinLengthDraft(submittedDraft);

    if (submittedPasswordMinLength == null) {
      setPasswordMinLengthDraft(String(currentSettings.passwordMinLength));
      passwordMinLengthSaveToastRequestedRef.current = false;
      toast.error(
        t("settings.securityPasswordMinLengthInvalid", {
          min: MIN_PASSWORD_LENGTH,
        }),
      );
      return;
    }

    if (submittedPasswordMinLength === currentSettings.passwordMinLength) {
      setPasswordMinLengthDraft(String(submittedPasswordMinLength));
      passwordMinLengthSaveToastRequestedRef.current = false;
      return;
    }

    setSaveBusy(true);
    try {
      const nextSettings = await applySecuritySettings(
        currentSettings.formLoginEnabled,
        submittedPasswordMinLength,
        currentSettings.skipLoginForLocalIps,
        currentSettings.mfaRequireConfigStepUp,
        currentSettings.mfaRequirePasswordLogin,
        currentSettings.totpRequireJellyfinLogin,
      );
      settingsRef.current = nextSettings;
      setSettings(nextSettings);
      setPasswordMinLengthDraft(String(nextSettings.passwordMinLength));
      if (passwordMinLengthSaveToastRequestedRef.current) {
        toast.success(t("settings.securityPreferenceSaved"));
      }
    } catch (error) {
      setPasswordMinLengthDraft(String(settingsRef.current.passwordMinLength));
      toast.error(errorMessage(error, t("settings.securitySaveFailed")));
    } finally {
      passwordMinLengthSaveToastRequestedRef.current = false;
      setSaveBusy(false);
    }
  }, [applySecuritySettings, t]);

  const submitPasswordMinLength = React.useCallback(async (
    showSuccessToast: boolean,
    draftOverride?: string,
  ): Promise<SecuritySettings | null> => {
    if (showSuccessToast) {
      passwordMinLengthSaveToastRequestedRef.current = true;
    }

    if (loading || confirmBusy) {
      return null;
    }

    const submittedDraft = draftOverride ?? passwordMinLengthDraft;
    await passwordMinLengthSaveQueueRef.current?.enqueue(
      submittedDraft,
      runPasswordMinLengthSave,
    );
    return settingsRef.current;
  }, [
    confirmBusy,
    loading,
    passwordMinLengthDraft,
    runPasswordMinLengthSave,
  ]);

  const handleToggle = React.useCallback((enabled: boolean) => {
    if (enabled === settings.formLoginEnabled || confirmBusy || saveBusy) {
      return;
    }

    if (enabled) {
      if (user?.hasPassword === false) {
        openAdminPasswordRequiredDialog();
        return;
      }

      setConfirmError(null);
      setConfirmPassword("");
      setEnableConfirmOpen(true);
      return;
    }

    setDisableConfirmOpen(true);
  }, [
    confirmBusy,
    openAdminPasswordRequiredDialog,
    saveBusy,
    settings.formLoginEnabled,
    user?.hasPassword,
  ]);

  const handleConfirmEnable = React.useCallback(async () => {
    const effectiveChangesNow =
      !settings.envOverrideActive && !settings.effectiveFormLoginEnabled;

    setConfirmBusy(true);
    setConfirmError(null);

    let verifiedSession: Awaited<ReturnType<typeof login>>;
    const currentUsername = user?.username.trim();
    if (!currentUsername) {
      setConfirmError(t("settings.securityCredentialsInvalid"));
      setConfirmBusy(false);
      return;
    }

    try {
      verifiedSession = await login(currentUsername, confirmPassword, {
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
        settings.mfaRequireConfigStepUp,
        settings.mfaRequirePasswordLogin,
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
      setConfirmPassword("");
      setConfirmError(null);
    } catch (error) {
      const saveErrorMessage = errorMessage(error, t("settings.securitySaveFailed"));
      if (saveErrorMessage === DEFAULT_ADMIN_PASSWORD_FORM_LOGIN_ERROR) {
        openAdminPasswordRequiredDialog();
        return;
      }

      setConfirmError(saveErrorMessage);
      toast.error(saveErrorMessage);
    } finally {
      setConfirmBusy(false);
    }
  }, [
    adoptSession,
    applySecuritySettings,
    confirmPassword,
    login,
    openAdminPasswordRequiredDialog,
    settings.effectiveFormLoginEnabled,
    settings.envOverrideActive,
    settings.skipLoginForLocalIps,
    submitPasswordMinLength,
    settings.mfaRequireConfigStepUp,
    settings.mfaRequirePasswordLogin,
    settings.totpRequireJellyfinLogin,
    t,
    effectivePasswordMinLength,
    user?.username,
  ]);

  const handleCancelEnable = React.useCallback(() => {
    if (confirmBusy) {
      return;
    }

    setEnableConfirmOpen(false);
    setConfirmPassword("");
    setConfirmError(null);
  }, [confirmBusy]);

  const handleConfirmAdminPasswordRequired = React.useCallback(() => {
    setAdminPasswordRequiredOpen(false);
    navigate("/settings/profile");
  }, [navigate]);

  const handleCancelAdminPasswordRequired = React.useCallback(() => {
    setAdminPasswordRequiredOpen(false);
  }, []);

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
        settings.mfaRequireConfigStepUp,
        settings.mfaRequirePasswordLogin,
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
    settings.mfaRequireConfigStepUp,
    settings.mfaRequirePasswordLogin,
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
        settings.mfaRequireConfigStepUp,
        settings.mfaRequirePasswordLogin,
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
    settings.mfaRequireConfigStepUp,
    settings.mfaRequirePasswordLogin,
    settings.totpRequireJellyfinLogin,
    t,
    token,
  ]);

  const handleMfaConfigStepUpChange = React.useCallback(async (enabled: boolean) => {
    if (confirmBusy || saveBusy || enabled === settings.mfaRequireConfigStepUp) {
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
        settings.mfaRequirePasswordLogin,
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
    settings.mfaRequireConfigStepUp,
    settings.mfaRequirePasswordLogin,
    settings.totpRequireJellyfinLogin,
    t,
  ]);

  const handleMfaPasswordLoginChange = React.useCallback(async (enabled: boolean) => {
    if (confirmBusy || saveBusy || enabled === settings.mfaRequirePasswordLogin) {
      return;
    }

    await submitPasswordMinLength(false);
    setSaveBusy(true);
    try {
      const nextSettings = await applySecuritySettings(
        settings.formLoginEnabled,
        effectivePasswordMinLength,
        settings.skipLoginForLocalIps,
        settings.mfaRequireConfigStepUp,
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
    settings.mfaRequireConfigStepUp,
    settings.mfaRequirePasswordLogin,
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
        settings.mfaRequireConfigStepUp,
        settings.mfaRequirePasswordLogin,
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
    settings.mfaRequireConfigStepUp,
    settings.mfaRequirePasswordLogin,
    settings.totpRequireJellyfinLogin,
    t,
  ]);

  const handleTotpEmbyLoginChange = React.useCallback(async (enabled: boolean) => {
    if (confirmBusy || saveBusy || enabled === settings.totpRequireEmbyLogin) {
      return;
    }

    await submitPasswordMinLength(false);
    setSaveBusy(true);
    try {
      const nextSettings = await applySecuritySettings(
        settings.formLoginEnabled,
        effectivePasswordMinLength,
        settings.skipLoginForLocalIps,
        settings.mfaRequireConfigStepUp,
        settings.mfaRequirePasswordLogin,
        settings.totpRequireJellyfinLogin,
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
    settings.mfaRequireConfigStepUp,
    settings.mfaRequirePasswordLogin,
    settings.totpRequireEmbyLogin,
    settings.totpRequireJellyfinLogin,
    t,
  ]);

  const handlePasswordMinLengthSubmit = React.useCallback(
    async (value?: string) => {
      await submitPasswordMinLength(true, value);
    },
    [submitPasswordMinLength],
  );

  return (
    <SettingsSecuritySection
      settings={settings}
      loading={loading || saveBusy}
      enableConfirmOpen={enableConfirmOpen}
      disableConfirmOpen={disableConfirmOpen}
      adminPasswordRequiredOpen={adminPasswordRequiredOpen}
      confirmBusy={confirmBusy}
      confirmPassword={confirmPassword}
      confirmError={confirmError}
      passwordMinLengthDraft={passwordMinLengthDraft}
      minPasswordLength={MIN_PASSWORD_LENGTH}
      onToggle={handleToggle}
      onConfirmPasswordChange={setConfirmPassword}
      onConfirmEnable={handleConfirmEnable}
      onCancelEnable={handleCancelEnable}
      onConfirmDisable={handleConfirmDisable}
      onCancelDisable={handleCancelDisable}
      onConfirmAdminPasswordRequired={handleConfirmAdminPasswordRequired}
      onCancelAdminPasswordRequired={handleCancelAdminPasswordRequired}
      onPasswordMinLengthDraftChange={setPasswordMinLengthDraft}
      onPasswordMinLengthSubmit={handlePasswordMinLengthSubmit}
      onSkipLocalIpsChange={handleSkipLocalIpsChange}
      onMfaConfigStepUpChange={handleMfaConfigStepUpChange}
      onMfaPasswordLoginChange={handleMfaPasswordLoginChange}
      onTotpJellyfinLoginChange={handleTotpJellyfinLoginChange}
      onTotpEmbyLoginChange={handleTotpEmbyLoginChange}
      externalAccountInvitesPanel={
        canManageExternalInvites ? (
          <ExternalAccountInvitesContainer showMediaServersLink />
        ) : null
      }
    />
  );
}
