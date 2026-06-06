import { useState, useCallback, useEffect, useMemo, type FormEvent } from "react";
import { useClient } from "urql";
import { notifyExternalAccountInviteSourcesChanged } from "@/components/containers/settings/external-account-invites-container";
import { sanitizeDigits } from "@/components/ui/input";
import { SettingsProfileSection } from "@/components/views/settings/settings-profile-section";
import {
  deleteMyPasskeyMutation,
  linkJellyfinAccountMutation,
  linkPlexAccountMutation,
  setUserPasswordMutation,
  totpDisableMutation,
  totpEnrollmentCompleteMutation,
  totpEnrollmentStartMutation,
  totpRegenerateRecoveryCodesMutation,
  mfaVerifyStepUpMutation,
  unlinkExternalAccountMutation,
} from "@/lib/graphql/mutations";
import {
  authProviderRuntimeSettingsQuery,
  linkedAccountsQuery,
  meQuery,
  myPasskeysQuery,
  myTotpQuery,
} from "@/lib/graphql/queries";
import {
  VISIBLE_EXTERNAL_ACCOUNT_PROVIDERS,
  isVisibleExternalAccountProvider,
} from "@/lib/constants/integration-providers";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useAuth, type AuthUser } from "@/lib/hooks/use-auth";
import type {
  AuthProviderConnection,
  AuthProviderSettings,
  ExternalAccountProvider,
  LinkedAccount,
  PasskeySummary,
  TotpEnrollmentComplete,
  TotpEnrollmentStart,
  TotpStatus,
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

const DEFAULT_AUTH_PROVIDER_SETTINGS: AuthProviderSettings = {
  allowedProviders: [],
  providerLoginEnabled: [],
  providerLinkingEnabled: [],
  allowedJellyfinConnectionIds: [],
  allowedPlexConnectionIds: [],
  allowedJellyfinConnections: [],
  allowedPlexConnections: [],
};

const TOTP_CODE_LENGTH = 6;

function sanitizeTotpCode(value: string): string {
  return sanitizeDigits(value).slice(0, TOTP_CODE_LENGTH);
}

function connectionsForProvider(
  settings: AuthProviderSettings,
  provider: ExternalAccountProvider,
): AuthProviderConnection[] {
  const connections =
    provider === "jellyfin"
      ? settings.allowedJellyfinConnections
      : settings.allowedPlexConnections;
  if (connections.length > 0) {
    return connections;
  }

  const ids =
    provider === "jellyfin"
      ? settings.allowedJellyfinConnectionIds
      : settings.allowedPlexConnectionIds;
  return ids.map((id) => ({
    id,
    displayName: id,
    userVisibleUrl: null,
    baseUrl: null,
    loginEnabled: settings.providerLoginEnabled.includes(provider),
    linkingEnabled: settings.providerLinkingEnabled.includes(provider),
  }));
}

function connectionLabelForDisplay(connection: AuthProviderConnection): string {
  return connection.userVisibleUrl
    ? `${connection.displayName} (${connection.userVisibleUrl})`
    : connection.displayName;
}

export function SettingsProfileContainer({ userId, username }: Props) {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const { adoptSession, login, user: authUser } = useAuth();
  const [currentPassword, setCurrentPassword] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [saving, setSaving] = useState(false);
  const [passkeys, setPasskeys] = useState<PasskeySummary[]>([]);
  const [hasPassword, setHasPassword] = useState<boolean | null>(null);
  const [accountKind, setAccountKind] = useState<UserAccountKind | null>(null);
  const [totpStatus, setTotpStatus] = useState<TotpStatus | null>(null);
  const [totpEnrollment, setTotpEnrollment] = useState<TotpEnrollmentStart | null>(null);
  const [totpEnrollmentCode, setTotpEnrollmentCode] = useState("");
  const [totpActionCode, setTotpActionCode] = useState("");
  const [totpRecoveryCodes, setTotpRecoveryCodes] = useState<string[]>([]);
  const [linkedAccounts, setLinkedAccounts] = useState<LinkedAccount[]>([]);
  const [authProviderSettings, setAuthProviderSettings] = useState<AuthProviderSettings>(
    DEFAULT_AUTH_PROVIDER_SETTINGS,
  );
  const [loadingPasskeys, setLoadingPasskeys] = useState(false);
  const [loadingTotp, setLoadingTotp] = useState(false);
  const [loadingLinkedAccounts, setLoadingLinkedAccounts] = useState(false);
  const [loadingLinkOptions, setLoadingLinkOptions] = useState(false);
  const [addingPasskey, setAddingPasskey] = useState(false);
  const [totpBusy, setTotpBusy] = useState(false);
  const [deletingPasskeyId, setDeletingPasskeyId] = useState<string | null>(null);
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

  const loadAuthProviderSettings = useCallback(async () => {
    if (!userId) {
      setAuthProviderSettings(DEFAULT_AUTH_PROVIDER_SETTINGS);
      setLoadingLinkOptions(false);
      return;
    }

    setLoadingLinkOptions(true);
    try {
      const result = await client
        .query<{ authProviderRuntimeSettings?: AuthProviderSettings }>(
          authProviderRuntimeSettingsQuery,
          {},
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (result.error) {
        setGlobalStatus(result.error.message);
        return;
      }
      setAuthProviderSettings(
        result.data?.authProviderRuntimeSettings ?? DEFAULT_AUTH_PROVIDER_SETTINGS,
      );
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("profile.linkAccountLoadFailed"));
    } finally {
      setLoadingLinkOptions(false);
    }
  }, [client, setGlobalStatus, t, userId]);

  useEffect(() => {
    void loadAuthProviderSettings();
  }, [loadAuthProviderSettings]);

  const linkableConnections = useMemo(() => {
    const linkedPairs = new Set(
      linkedAccounts.map((account) => `${account.provider}:${account.connectionId}`),
    );

    const eligibleForProvider = (provider: ExternalAccountProvider) => {
      if (
        !isVisibleExternalAccountProvider(provider) ||
        !authProviderSettings.allowedProviders.includes(provider) ||
        !authProviderSettings.providerLinkingEnabled.includes(provider)
      ) {
        return [];
      }

      return connectionsForProvider(authProviderSettings, provider).filter(
        (connection) =>
          connection.linkingEnabled && !linkedPairs.has(`${provider}:${connection.id}`),
      );
    };

    return {
      jellyfin: eligibleForProvider("jellyfin"),
      plex: eligibleForProvider("plex"),
    };
  }, [authProviderSettings, linkedAccounts]);

  const linkedAccountConnectionLabels = useMemo(() => {
    const labels: Record<string, string> = {};
    VISIBLE_EXTERNAL_ACCOUNT_PROVIDERS.forEach((provider) => {
      connectionsForProvider(authProviderSettings, provider).forEach((connection) => {
        labels[`${provider}:${connection.id}`] = connectionLabelForDisplay(connection);
      });
    });
    return labels;
  }, [authProviderSettings]);

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
        .mutation<{ deleteMyPasskey?: boolean }, { id: string }>(deleteMyPasskeyMutation, { id })
        .toPromise();
      if (result.error || result.data?.deleteMyPasskey !== true) {
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

  const handleVerifyTotpStepUp = useCallback(async () => {
    if (totpActionCode.length !== TOTP_CODE_LENGTH) return;

    setTotpBusy(true);
    try {
      const result = await client
        .mutation<
          { mfaVerifyStepUp?: { token: string; user: AuthUser | null } },
          { input: { code: string } }
        >(mfaVerifyStepUpMutation, { input: { code: totpActionCode } })
        .toPromise();
      if (result.error || !result.data?.mfaVerifyStepUp) {
        setGlobalStatus(result.error?.message ?? t("profile.totpOperationFailed"));
        return;
      }
      adoptSession(result.data.mfaVerifyStepUp.token, result.data.mfaVerifyStepUp.user);
      setTotpActionCode("");
      setGlobalStatus(t("profile.mfaStepUpVerified"));
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("profile.totpOperationFailed"));
    } finally {
      setTotpBusy(false);
    }
  }, [adoptSession, client, setGlobalStatus, t, totpActionCode]);

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
        .mutation<{ unlinkExternalAccount?: boolean }, { input: { linkedAccountId: string } }>(
          unlinkExternalAccountMutation,
          { input: { linkedAccountId: id } },
        )
        .toPromise();
      if (result.error || result.data?.unlinkExternalAccount !== true) {
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
      loadingTotp={loadingTotp}
      loadingLinkedAccounts={loadingLinkedAccounts}
      loadingLinkOptions={loadingLinkOptions}
      addingPasskey={addingPasskey}
      totpBusy={totpBusy}
      deletingPasskeyId={deletingPasskeyId}
      unlinkingAccountId={unlinkingAccountId}
      onAddPasskey={handleAddPasskey}
      onDeletePasskey={handleDeletePasskey}
      onStartTotpEnrollment={handleStartTotpEnrollment}
      onTotpEnrollmentCodeChange={(value) => setTotpEnrollmentCode(sanitizeTotpCode(value))}
      onCompleteTotpEnrollment={handleCompleteTotpEnrollment}
      onTotpActionCodeChange={(value) => setTotpActionCode(sanitizeTotpCode(value))}
      onVerifyTotpStepUp={handleVerifyTotpStepUp}
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
