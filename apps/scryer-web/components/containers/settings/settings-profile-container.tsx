import { useState, useCallback, useEffect, useMemo, type FormEvent } from "react";
import { useClient } from "urql";
import { notifyExternalAccountInviteSourcesChanged } from "@/components/containers/settings/external-account-invites-container";
import { sanitizeTotpCode } from "@/components/auth/totp-code-form";
import { SettingsProfileSection } from "@/components/views/settings/settings-profile-section";
import {
  deleteMyPasskeyMutation,
  linkJellyfinAccountMutation,
  linkPlexAccountMutation,
  revokeMyOauthAppMutation,
  setMyUiSettingsMutation,
  setUserPasswordMutation,
  totpDisableMutation,
  totpEnrollmentCompleteMutation,
  totpEnrollmentStartMutation,
  totpRegenerateRecoveryCodesMutation,
  unlinkExternalAccountMutation,
} from "@/lib/graphql/mutations";
import {
  externalAuthRuntimeSettingsQuery,
  linkedAccountsQuery,
  meQuery,
  myOauthAppsQuery,
  myPasskeysQuery,
  myTotpQuery,
} from "@/lib/graphql/queries";
import { normalizeGraphQlErrorMessage } from "@/lib/graphql/error-message";
import {
  VISIBLE_EXTERNAL_ACCOUNT_PROVIDERS,
  isVisibleExternalAccountProvider,
} from "@/lib/constants/integration-providers";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import {
  useUiSettings,
  uiSettingsInputFromSettings,
} from "@/lib/context/ui-settings-context";
import { useAuth, type AuthUser } from "@/lib/hooks/use-auth";
import type {
  ExternalAccountProvider,
  ExternalAuthRuntimeConnection,
  ExternalAuthRuntimeSettings,
  LinkedAccount,
  OAuthConnectedApp,
  PasskeySummary,
  TotpEnrollmentComplete,
  TotpEnrollmentStart,
  TotpStatus,
  UiSettings,
} from "@/lib/types/settings";
import type { UserAccountKind } from "@/lib/types/users";
import { PasskeyClientError, registerPasskey } from "@/lib/utils/passkeys";
import { authenticateWithPlexPin } from "@/lib/utils/plex-oauth";

type Props = {
  userId?: string;
  username?: string;
};

type LinkAccountDraft = {
  provider: ExternalAccountProvider;
  connectionId: string;
  jellyfinUsername: string;
  jellyfinPassword: string;
};

const DEFAULT_EXTERNAL_AUTH_RUNTIME_SETTINGS: ExternalAuthRuntimeSettings = {
  loginProviders: [],
  linkingProviders: [],
  connections: [],
};

const TOTP_CODE_LENGTH = 6;
const PASSKEY_FORM_LOGIN_DISABLED_ERROR =
  "passkey authentication is unavailable while form login is disabled";

function connectionsForProvider(
  settings: ExternalAuthRuntimeSettings,
  provider: ExternalAccountProvider,
): ExternalAuthRuntimeConnection[] {
  return settings.connections.filter((connection) => connection.provider === provider);
}

function connectionLabelForDisplay(connection: ExternalAuthRuntimeConnection): string {
  return connection.displayName;
}

function isPasskeyFormLoginDisabledError(message: string): boolean {
  return normalizeGraphQlErrorMessage(message) === PASSKEY_FORM_LOGIN_DISABLED_ERROR;
}

export function SettingsProfileContainer({ userId, username }: Props) {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const { login, user: authUser } = useAuth();
  const { uiSettings, setUiSettings, uiSettingsLoaded, uiSettingsLoading } =
    useUiSettings();
  const [savingHighlightColor, setSavingHighlightColor] = useState<string | null>(
    null,
  );
  const [currentPassword, setCurrentPassword] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [saving, setSaving] = useState(false);
  const [passkeys, setPasskeys] = useState<PasskeySummary[]>([]);
  const [oauthApps, setOauthApps] = useState<OAuthConnectedApp[]>([]);
  const [hasPassword, setHasPassword] = useState<boolean | null>(null);
  const [accountKind, setAccountKind] = useState<UserAccountKind | null>(null);
  const [totpStatus, setTotpStatus] = useState<TotpStatus | null>(null);
  const [totpEnrollment, setTotpEnrollment] = useState<TotpEnrollmentStart | null>(null);
  const [totpEnrollmentCode, setTotpEnrollmentCode] = useState("");
  const [totpActionCode, setTotpActionCode] = useState("");
  const [totpRecoveryCodes, setTotpRecoveryCodes] = useState<string[]>([]);
  const [linkedAccounts, setLinkedAccounts] = useState<LinkedAccount[]>([]);
  const [externalAuthSettings, setExternalAuthSettings] = useState<ExternalAuthRuntimeSettings>(
    DEFAULT_EXTERNAL_AUTH_RUNTIME_SETTINGS,
  );
  const [loadingPasskeys, setLoadingPasskeys] = useState(false);
  const [loadingOauthApps, setLoadingOauthApps] = useState(false);
  const [loadingTotp, setLoadingTotp] = useState(false);
  const [loadingLinkedAccounts, setLoadingLinkedAccounts] = useState(false);
  const [loadingLinkOptions, setLoadingLinkOptions] = useState(false);
  const [addingPasskey, setAddingPasskey] = useState(false);
  const [totpBusy, setTotpBusy] = useState(false);
  const [deletingPasskeyId, setDeletingPasskeyId] = useState<string | null>(null);
  const [revokingOauthGrantId, setRevokingOauthGrantId] = useState<string | null>(null);
  const [unlinkingAccountId, setUnlinkingAccountId] = useState<string | null>(null);
  const [linkingProvider, setLinkingProvider] = useState<ExternalAccountProvider | null>(null);
  const [linkAccountDraft, setLinkAccountDraft] = useState<LinkAccountDraft>({
    provider: "jellyfin",
    connectionId: "",
    jellyfinUsername: "",
    jellyfinPassword: "",
  });
  const [linkAccountBusy, setLinkAccountBusy] = useState(false);
  const [linkAccountError, setLinkAccountError] = useState<string | null>(null);

  const formatPasskeyError = useCallback((error: unknown) => {
    if (error instanceof PasskeyClientError) {
      if (error.code === "unsupported") {
        return t("auth.passkeyUnsupported");
      }
      if (error.code === "cancelled") {
        return t("auth.passkeyCancelled");
      }
      return error.message || t("profile.passkeyOperationFailed");
    }

    return error instanceof Error ? error.message : t("profile.passkeyOperationFailed");
  }, [t]);

  const handleChangePassword = useCallback(async () => {
    if (!userId || !newPassword || newPassword !== confirmPassword) return;

    setSaving(true);
    try {
      const result = await client
        .mutation(setUserPasswordMutation, {
          input: {
            userId,
            password: newPassword,
            currentPassword: hasPassword === true ? currentPassword : undefined,
          },
        })
        .toPromise();

      if (result.error) {
        setGlobalStatus(result.error.message);
        return;
      }

      setHasPassword(result.data?.setUserPassword?.hasPassword === true);
      setAccountKind(result.data?.setUserPassword?.accountKind ?? accountKind);
      const profileUsername = username ?? authUser?.username;
      if (profileUsername) {
        const refreshed = await login(profileUsername, newPassword);
        setHasPassword(refreshed.user?.hasPassword === true);
        setAccountKind(refreshed.user?.accountKind ?? accountKind);
      }
      setCurrentPassword("");
      setNewPassword("");
      setConfirmPassword("");
      setGlobalStatus(t("profile.passwordUpdated"));
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setSaving(false);
    }
  }, [accountKind, authUser?.username, client, userId, username, hasPassword, currentPassword, newPassword, confirmPassword, login, setGlobalStatus, t]);

  const handleSelectHighlightColor = useCallback(
    async (highlightColor: string) => {
      if (
        !uiSettingsLoaded ||
        uiSettingsLoading ||
        savingHighlightColor ||
        uiSettings.highlightColor === highlightColor
      ) {
        return;
      }

      const previous = uiSettings;
      const next: UiSettings = { ...uiSettings, highlightColor };
      setSavingHighlightColor(highlightColor);
      // Apply the accent live while the mutation persists through the interface.
      setUiSettings(next);
      try {
        const result = await client
          .mutation<{ setMyUiSettings?: UiSettings }, { input: UiSettings }>(
            setMyUiSettingsMutation,
            { input: uiSettingsInputFromSettings(next) },
          )
          .toPromise();
        if (result.error || !result.data?.setMyUiSettings) {
          setUiSettings(previous);
          setGlobalStatus(
            result.error?.message ?? t("profile.highlightColorSaveFailed"),
          );
          return;
        }
        setUiSettings(result.data.setMyUiSettings);
        setGlobalStatus(t("profile.highlightColorSaved"));
      } catch (error) {
        setUiSettings(previous);
        setGlobalStatus(
          error instanceof Error
            ? error.message
            : t("profile.highlightColorSaveFailed"),
        );
      } finally {
        setSavingHighlightColor(null);
      }
    },
    [
      client,
      savingHighlightColor,
      setGlobalStatus,
      setUiSettings,
      t,
      uiSettings,
      uiSettingsLoaded,
      uiSettingsLoading,
    ],
  );

  useEffect(() => {
    if (!userId) {
      setHasPassword(false);
      setAccountKind(null);
      return;
    }

    let cancelled = false;

    (async () => {
      setHasPassword(null);
      setAccountKind(null);
      try {
        const result = await client
          .query<{ me?: Pick<AuthUser, "id" | "hasPassword" | "accountKind"> | null }>(
            meQuery,
            {},
          )
          .toPromise();
        if (cancelled) return;

        if (result.error) {
          setGlobalStatus(result.error.message);
          setHasPassword(false);
          setAccountKind(null);
          return;
        }

        const currentUser = result.data?.me;
        const isProfileUser = currentUser?.id === userId;
        setHasPassword(isProfileUser && currentUser.hasPassword === true);
        setAccountKind(isProfileUser ? currentUser.accountKind ?? "local" : null);
      } catch (error) {
        if (!cancelled) {
          setHasPassword(false);
          setAccountKind(null);
          setGlobalStatus(error instanceof Error ? error.message : t("profile.passkeyOperationFailed"));
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [client, setGlobalStatus, t, userId]);

  const loadPasskeys = useCallback(async () => {
    if (!userId || accountKind !== "local") {
      setPasskeys([]);
      setLoadingPasskeys(false);
      return;
    }

    setLoadingPasskeys(true);
    try {
      const result = await client.query<{ myPasskeys?: PasskeySummary[] }>(myPasskeysQuery, {}).toPromise();
      if (result.error) {
        if (isPasskeyFormLoginDisabledError(result.error.message)) {
          setPasskeys([]);
          return;
        }

        setGlobalStatus(result.error.message);
        return;
      }

      setPasskeys(result.data?.myPasskeys ?? []);
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("profile.passkeyOperationFailed"));
    } finally {
      setLoadingPasskeys(false);
    }
  }, [accountKind, client, setGlobalStatus, t, userId]);

  useEffect(() => {
    void loadPasskeys();
  }, [loadPasskeys]);

  const loadOauthApps = useCallback(async () => {
    if (!userId) {
      setOauthApps([]);
      setLoadingOauthApps(false);
      return;
    }

    setLoadingOauthApps(true);
    try {
      const result = await client
        .query<{ myOauthApps?: OAuthConnectedApp[] }>(myOauthAppsQuery, {})
        .toPromise();
      if (result.error) {
        setGlobalStatus(result.error.message);
        return;
      }
      setOauthApps(result.data?.myOauthApps ?? []);
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : "Connected apps could not be loaded.");
    } finally {
      setLoadingOauthApps(false);
    }
  }, [client, setGlobalStatus, userId]);

  useEffect(() => {
    void loadOauthApps();
  }, [loadOauthApps]);

  const loadTotp = useCallback(async () => {
    if (!userId) {
      setTotpStatus(null);
      setLoadingTotp(false);
      return;
    }

    setLoadingTotp(true);
    try {
      const result = await client.query<{ myTotp?: TotpStatus }>(myTotpQuery, {}).toPromise();
      if (result.error) {
        setGlobalStatus(result.error.message);
        return;
      }
      setTotpStatus(result.data?.myTotp ?? null);
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("profile.totpOperationFailed"));
    } finally {
      setLoadingTotp(false);
    }
  }, [client, setGlobalStatus, t, userId]);

  useEffect(() => {
    void loadTotp();
  }, [loadTotp]);

  const loadLinkedAccounts = useCallback(async () => {
    if (!userId) {
      setLinkedAccounts([]);
      setLoadingLinkedAccounts(false);
      return;
    }

    setLoadingLinkedAccounts(true);
    try {
      const result = await client
        .query<{ linkedAccounts?: LinkedAccount[] }>(linkedAccountsQuery, { userId })
        .toPromise();
      if (result.error) {
        setGlobalStatus(result.error.message);
        return;
      }
      setLinkedAccounts(
        (result.data?.linkedAccounts ?? []).filter((account) =>
          isVisibleExternalAccountProvider(account.provider),
        ),
      );
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("profile.linkedAccountsLoadFailed"));
    } finally {
      setLoadingLinkedAccounts(false);
    }
  }, [client, setGlobalStatus, t, userId]);

  useEffect(() => {
    void loadLinkedAccounts();
  }, [loadLinkedAccounts]);

  const loadExternalAuthSettings = useCallback(async () => {
    if (!userId) {
      setExternalAuthSettings(DEFAULT_EXTERNAL_AUTH_RUNTIME_SETTINGS);
      setLoadingLinkOptions(false);
      return;
    }

    setLoadingLinkOptions(true);
    try {
      const result = await client
        .query<{ externalAuthRuntimeSettings?: ExternalAuthRuntimeSettings }>(
          externalAuthRuntimeSettingsQuery,
          {},
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (result.error) {
        setGlobalStatus(result.error.message);
        return;
      }
      setExternalAuthSettings(
        result.data?.externalAuthRuntimeSettings ?? DEFAULT_EXTERNAL_AUTH_RUNTIME_SETTINGS,
      );
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("profile.linkAccountLoadFailed"));
    } finally {
      setLoadingLinkOptions(false);
    }
  }, [client, setGlobalStatus, t, userId]);

  useEffect(() => {
    void loadExternalAuthSettings();
  }, [loadExternalAuthSettings]);

  const linkableConnections = useMemo(() => {
    const linkedPairs = new Set(
      linkedAccounts.map((account) => `${account.provider}:${account.connectionId}`),
    );

    const eligibleForProvider = (provider: ExternalAccountProvider) => {
      if (
        !isVisibleExternalAccountProvider(provider) ||
        !externalAuthSettings.linkingProviders.includes(provider)
      ) {
        return [];
      }

      return connectionsForProvider(externalAuthSettings, provider).filter(
        (connection) =>
          connection.linkingEnabled && !linkedPairs.has(`${provider}:${connection.id}`),
      );
    };

    return {
      jellyfin: eligibleForProvider("jellyfin"),
      plex: eligibleForProvider("plex"),
    };
  }, [externalAuthSettings, linkedAccounts]);

  const linkedAccountConnectionLabels = useMemo(() => {
    const labels: Record<string, string> = {};
    VISIBLE_EXTERNAL_ACCOUNT_PROVIDERS.forEach((provider) => {
      connectionsForProvider(externalAuthSettings, provider).forEach((connection) => {
        labels[`${provider}:${connection.id}`] = connectionLabelForDisplay(connection);
      });
    });
    return labels;
  }, [externalAuthSettings]);

  const handleAddPasskey = useCallback(async () => {
    if (!userId || hasPassword !== true || accountKind !== "local") return;

    setAddingPasskey(true);
    try {
      const passkey = await registerPasskey(client);
      setPasskeys((current) => [...current, passkey]);
      setGlobalStatus(t("profile.passkeyAdded"));
    } catch (error) {
      setGlobalStatus(formatPasskeyError(error));
    } finally {
      setAddingPasskey(false);
    }
  }, [accountKind, client, formatPasskeyError, hasPassword, setGlobalStatus, t, userId]);

  const handleDeletePasskey = useCallback(async (id: string) => {
    setDeletingPasskeyId(id);
    try {
      const result = await client
        .mutation<{ deleteMyPasskey?: { deleted?: boolean } }, { id: string }>(
          deleteMyPasskeyMutation,
          { id },
        )
        .toPromise();
      if (result.error || result.data?.deleteMyPasskey?.deleted !== true) {
        setGlobalStatus(result.error?.message ?? t("profile.passkeyDeleteFailed"));
        return;
      }

      setPasskeys((current) => current.filter((passkey) => passkey.id !== id));
      setGlobalStatus(t("profile.passkeyDeleted"));
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("profile.passkeyDeleteFailed"));
    } finally {
      setDeletingPasskeyId(null);
    }
  }, [client, setGlobalStatus, t]);

  const handleRevokeOauthApp = useCallback(async (grantId: string) => {
    setRevokingOauthGrantId(grantId);
    try {
      const result = await client
        .mutation<{ revokeMyOauthApp?: { revoked?: boolean } }, { grantId: string }>(
          revokeMyOauthAppMutation,
          { grantId },
        )
        .toPromise();
      if (result.error || result.data?.revokeMyOauthApp?.revoked !== true) {
        setGlobalStatus(result.error?.message ?? "Connected app could not be revoked.");
        return;
      }

      setOauthApps((current) => current.filter((app) => app.grantId !== grantId));
      setGlobalStatus("Connected app revoked.");
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : "Connected app could not be revoked.");
    } finally {
      setRevokingOauthGrantId(null);
    }
  }, [client, setGlobalStatus]);

  const handleStartTotpEnrollment = useCallback(async () => {
    setTotpBusy(true);
    setTotpRecoveryCodes([]);
    try {
      const result = await client
        .mutation<{ totpEnrollmentStart?: TotpEnrollmentStart }>(totpEnrollmentStartMutation, {})
        .toPromise();
      if (result.error || !result.data?.totpEnrollmentStart) {
        setGlobalStatus(result.error?.message ?? t("profile.totpOperationFailed"));
        return;
      }
      setTotpEnrollment(result.data.totpEnrollmentStart);
      setTotpEnrollmentCode("");
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("profile.totpOperationFailed"));
    } finally {
      setTotpBusy(false);
    }
  }, [client, setGlobalStatus, t]);

  const handleCompleteTotpEnrollment = useCallback(async () => {
    if (!totpEnrollment || totpEnrollmentCode.length !== TOTP_CODE_LENGTH) return;

    setTotpBusy(true);
    try {
      const result = await client
        .mutation<
          { totpEnrollmentComplete?: TotpEnrollmentComplete },
          { input: { challengeId: string; code: string } }
        >(totpEnrollmentCompleteMutation, {
          input: {
            challengeId: totpEnrollment.challengeId,
            code: totpEnrollmentCode,
          },
        })
        .toPromise();
      if (result.error || !result.data?.totpEnrollmentComplete) {
        setGlobalStatus(result.error?.message ?? t("profile.totpOperationFailed"));
        return;
      }
      setTotpStatus(result.data.totpEnrollmentComplete.status);
      setTotpRecoveryCodes(result.data.totpEnrollmentComplete.recoveryCodes);
      setTotpEnrollment(null);
      setTotpEnrollmentCode("");
      setGlobalStatus(t("profile.totpEnabled"));
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("profile.totpOperationFailed"));
    } finally {
      setTotpBusy(false);
    }
  }, [client, setGlobalStatus, t, totpEnrollment, totpEnrollmentCode]);

  const handleDisableTotp = useCallback(async () => {
    if (totpActionCode.length !== TOTP_CODE_LENGTH) return;

    setTotpBusy(true);
    try {
      const result = await client
        .mutation<{ totpDisable?: TotpStatus }, { input: { code: string } }>(
          totpDisableMutation,
          { input: { code: totpActionCode } },
        )
        .toPromise();
      if (result.error || !result.data?.totpDisable) {
        setGlobalStatus(result.error?.message ?? t("profile.totpOperationFailed"));
        return;
      }
      setTotpStatus(result.data.totpDisable);
      setTotpActionCode("");
      setTotpRecoveryCodes([]);
      setGlobalStatus(t("profile.totpDisabled"));
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("profile.totpOperationFailed"));
    } finally {
      setTotpBusy(false);
    }
  }, [client, setGlobalStatus, t, totpActionCode]);

  const handleRegenerateTotpRecoveryCodes = useCallback(async () => {
    if (totpActionCode.length !== TOTP_CODE_LENGTH) return;

    setTotpBusy(true);
    try {
      const result = await client
        .mutation<
          { totpRegenerateRecoveryCodes?: TotpEnrollmentComplete },
          { input: { code: string } }
        >(totpRegenerateRecoveryCodesMutation, { input: { code: totpActionCode } })
        .toPromise();
      if (result.error || !result.data?.totpRegenerateRecoveryCodes) {
        setGlobalStatus(result.error?.message ?? t("profile.totpOperationFailed"));
        return;
      }
      setTotpStatus(result.data.totpRegenerateRecoveryCodes.status);
      setTotpRecoveryCodes(result.data.totpRegenerateRecoveryCodes.recoveryCodes);
      setTotpActionCode("");
      setGlobalStatus(t("profile.totpRecoveryCodesRegenerated"));
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("profile.totpOperationFailed"));
    } finally {
      setTotpBusy(false);
    }
  }, [client, setGlobalStatus, t, totpActionCode]);

  const handleUnlinkExternalAccount = useCallback(async (id: string) => {
    setUnlinkingAccountId(id);
    try {
      const result = await client
        .mutation<{ unlinkExternalAccount?: { unlinked?: boolean } }, { linkedAccountId: string }>(
          unlinkExternalAccountMutation,
          { linkedAccountId: id },
        )
        .toPromise();
      if (result.error || result.data?.unlinkExternalAccount?.unlinked !== true) {
        setGlobalStatus(result.error?.message ?? t("profile.linkedAccountUnlinkFailed"));
        return;
      }

      setLinkedAccounts((current) => current.filter((account) => account.id !== id));
      notifyExternalAccountInviteSourcesChanged();
      setGlobalStatus(t("profile.linkedAccountUnlinked"));
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("profile.linkedAccountUnlinkFailed"),
      );
    } finally {
      setUnlinkingAccountId(null);
    }
  }, [client, setGlobalStatus, t]);

  const handleStartLinkAccount = useCallback((provider: ExternalAccountProvider) => {
    if (!isVisibleExternalAccountProvider(provider)) {
      return;
    }
    const connections = provider === "jellyfin" ? linkableConnections.jellyfin : linkableConnections.plex;
    setLinkingProvider(provider);
    setLinkAccountError(null);
    setLinkAccountDraft({
      provider,
      connectionId: connections[0]?.id ?? "",
      jellyfinUsername: "",
      jellyfinPassword: "",
    });
  }, [linkableConnections.jellyfin, linkableConnections.plex]);

  const handleCancelLinkAccount = useCallback(() => {
    setLinkingProvider(null);
    setLinkAccountError(null);
    setLinkAccountDraft((current) => ({
      ...current,
      jellyfinPassword: "",
    }));
  }, []);

  const handleLinkAccountConnectionChange = useCallback((connectionId: string) => {
    setLinkAccountDraft((current) => ({ ...current, connectionId }));
  }, []);

  const handleLinkAccountUsernameChange = useCallback((jellyfinUsername: string) => {
    setLinkAccountDraft((current) => ({ ...current, jellyfinUsername }));
  }, []);

  const handleLinkAccountPasswordChange = useCallback((jellyfinPassword: string) => {
    setLinkAccountDraft((current) => ({ ...current, jellyfinPassword }));
  }, []);

  const handleSubmitJellyfinLink = useCallback(async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (
      linkingProvider !== "jellyfin" ||
      !linkAccountDraft.connectionId ||
      !linkAccountDraft.jellyfinUsername.trim() ||
      !linkAccountDraft.jellyfinPassword
    ) {
      return;
    }

    setLinkAccountBusy(true);
    setLinkAccountError(null);
    try {
      const result = await client
        .mutation<{ linkJellyfinAccount?: LinkedAccount }, {
          input: { connectionId: string; username: string; password: string };
        }>(linkJellyfinAccountMutation, {
          input: {
            connectionId: linkAccountDraft.connectionId,
            username: linkAccountDraft.jellyfinUsername.trim(),
            password: linkAccountDraft.jellyfinPassword,
          },
        })
        .toPromise();

      if (result.error || !result.data?.linkJellyfinAccount) {
        const message = result.error?.message ?? t("profile.linkAccountFailed");
        setLinkAccountError(message);
        setGlobalStatus(message);
        return;
      }

      setLinkingProvider(null);
      setLinkAccountDraft((current) => ({ ...current, jellyfinPassword: "" }));
      await loadLinkedAccounts();
      notifyExternalAccountInviteSourcesChanged();
      setGlobalStatus(t("profile.linkAccountLinked"));
    } catch (error) {
      const message = error instanceof Error ? error.message : t("profile.linkAccountFailed");
      setLinkAccountError(message);
      setGlobalStatus(message);
    } finally {
      setLinkAccountDraft((current) => ({ ...current, jellyfinPassword: "" }));
      setLinkAccountBusy(false);
    }
  }, [client, linkAccountDraft, linkingProvider, loadLinkedAccounts, setGlobalStatus, t]);

  const handleSubmitPlexLink = useCallback(async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (linkingProvider !== "plex" || !linkAccountDraft.connectionId) {
      return;
    }

    setLinkAccountBusy(true);
    setLinkAccountError(null);
    try {
      setGlobalStatus(t("auth.plexPinFlowPending"));
      const plexAuthToken = await authenticateWithPlexPin();
      const result = await client
        .mutation<{ linkPlexAccount?: LinkedAccount }, {
          input: { connectionId: string; plexAuthToken: string };
        }>(linkPlexAccountMutation, {
          input: {
            connectionId: linkAccountDraft.connectionId,
            plexAuthToken,
          },
        })
        .toPromise();

      if (result.error || !result.data?.linkPlexAccount) {
        const message = result.error?.message ?? t("profile.linkAccountFailed");
        setLinkAccountError(message);
        setGlobalStatus(message);
        return;
      }

      setLinkingProvider(null);
      await loadLinkedAccounts();
      notifyExternalAccountInviteSourcesChanged();
      setGlobalStatus(t("profile.linkAccountLinked"));
    } catch (error) {
      const message = error instanceof Error ? error.message : t("profile.linkAccountFailed");
      setLinkAccountError(message);
      setGlobalStatus(message);
    } finally {
      setLinkAccountBusy(false);
    }
  }, [client, linkAccountDraft.connectionId, linkingProvider, loadLinkedAccounts, setGlobalStatus, t]);

  return (
    <SettingsProfileSection
      username={username}
      highlightColor={uiSettings.highlightColor}
      savingHighlightColor={savingHighlightColor}
      onSelectHighlightColor={handleSelectHighlightColor}
      currentPassword={currentPassword}
      newPassword={newPassword}
      confirmPassword={confirmPassword}
      saving={saving}
      canChangePassword={accountKind === "local"}
      requiresCurrentPassword={hasPassword === true}
      onCurrentPasswordChange={setCurrentPassword}
      onNewPasswordChange={setNewPassword}
      onConfirmPasswordChange={setConfirmPassword}
      onChangePassword={handleChangePassword}
      showPasskeys={Boolean(userId) && accountKind === "local"}
      canAddPasskey={accountKind === "local" && hasPassword === true}
      passkeys={passkeys}
      oauthApps={oauthApps}
      totpStatus={totpStatus}
      totpEnrollment={totpEnrollment}
      totpEnrollmentCode={totpEnrollmentCode}
      totpActionCode={totpActionCode}
      totpRecoveryCodes={totpRecoveryCodes}
      linkedAccounts={linkedAccounts}
      linkedAccountConnectionLabels={linkedAccountConnectionLabels}
      linkableJellyfinConnections={linkableConnections.jellyfin}
      linkablePlexConnections={linkableConnections.plex}
      linkingProvider={linkingProvider}
      linkAccountConnectionId={linkAccountDraft.connectionId}
      linkAccountUsername={linkAccountDraft.jellyfinUsername}
      linkAccountPassword={linkAccountDraft.jellyfinPassword}
      linkAccountBusy={linkAccountBusy}
      linkAccountError={linkAccountError}
      loadingPasskeys={loadingPasskeys}
      loadingOauthApps={loadingOauthApps}
      loadingTotp={loadingTotp}
      loadingLinkedAccounts={loadingLinkedAccounts}
      loadingLinkOptions={loadingLinkOptions}
      addingPasskey={addingPasskey}
      totpBusy={totpBusy}
      deletingPasskeyId={deletingPasskeyId}
      revokingOauthGrantId={revokingOauthGrantId}
      unlinkingAccountId={unlinkingAccountId}
      onAddPasskey={handleAddPasskey}
      onDeletePasskey={handleDeletePasskey}
      onRevokeOauthApp={handleRevokeOauthApp}
      onStartTotpEnrollment={handleStartTotpEnrollment}
      onTotpEnrollmentCodeChange={(value) => setTotpEnrollmentCode(sanitizeTotpCode(value))}
      onCompleteTotpEnrollment={handleCompleteTotpEnrollment}
      onTotpActionCodeChange={(value) => setTotpActionCode(sanitizeTotpCode(value))}
      onDisableTotp={handleDisableTotp}
      onRegenerateTotpRecoveryCodes={handleRegenerateTotpRecoveryCodes}
      onStartLinkAccount={handleStartLinkAccount}
      onCancelLinkAccount={handleCancelLinkAccount}
      onLinkAccountConnectionChange={handleLinkAccountConnectionChange}
      onLinkAccountUsernameChange={handleLinkAccountUsernameChange}
      onLinkAccountPasswordChange={handleLinkAccountPasswordChange}
      onSubmitJellyfinLink={handleSubmitJellyfinLink}
      onSubmitPlexLink={handleSubmitPlexLink}
      onUnlinkExternalAccount={handleUnlinkExternalAccount}
    />
  );
}
