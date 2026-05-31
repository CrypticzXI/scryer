import { useState, useCallback, useEffect } from "react";
import { useClient } from "urql";
import { SettingsProfileSection } from "@/components/views/settings/settings-profile-section";
import {
  deleteMyPasskeyMutation,
  setUserPasswordMutation,
  totpDisableMutation,
  totpEnrollmentCompleteMutation,
  totpEnrollmentStartMutation,
  totpRegenerateRecoveryCodesMutation,
  totpVerifyStepUpMutation,
  unlinkExternalAccountMutation,
} from "@/lib/graphql/mutations";
import { linkedAccountsQuery, meQuery, myPasskeysQuery, myTotpQuery } from "@/lib/graphql/queries";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useAuth, type AuthUser } from "@/lib/hooks/use-auth";
import type {
  LinkedAccount,
  PasskeySummary,
  TotpEnrollmentComplete,
  TotpEnrollmentStart,
  TotpStatus,
} from "@/lib/types/settings";
import type { UserAccountKind } from "@/lib/types/users";
import { PasskeyClientError, registerPasskey } from "@/lib/utils/passkeys";

type Props = {
  userId?: string;
  username?: string;
};

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
  const [loadingPasskeys, setLoadingPasskeys] = useState(false);
  const [loadingTotp, setLoadingTotp] = useState(false);
  const [loadingLinkedAccounts, setLoadingLinkedAccounts] = useState(false);
  const [addingPasskey, setAddingPasskey] = useState(false);
  const [totpBusy, setTotpBusy] = useState(false);
  const [deletingPasskeyId, setDeletingPasskeyId] = useState<string | null>(null);
  const [unlinkingAccountId, setUnlinkingAccountId] = useState<string | null>(null);

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
      setLinkedAccounts(result.data?.linkedAccounts ?? []);
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("profile.linkedAccountsLoadFailed"));
    } finally {
      setLoadingLinkedAccounts(false);
    }
  }, [client, setGlobalStatus, t, userId]);

  useEffect(() => {
    void loadLinkedAccounts();
  }, [loadLinkedAccounts]);

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
    if (!totpEnrollment || !totpEnrollmentCode.trim()) return;

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
    if (!totpActionCode.trim()) return;

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
    if (!totpActionCode.trim()) return;

    setTotpBusy(true);
    try {
      const result = await client
        .mutation<
          { totpVerifyStepUp?: { token: string; user: AuthUser | null } },
          { input: { code: string } }
        >(totpVerifyStepUpMutation, { input: { code: totpActionCode } })
        .toPromise();
      if (result.error || !result.data?.totpVerifyStepUp) {
        setGlobalStatus(result.error?.message ?? t("profile.totpOperationFailed"));
        return;
      }
      adoptSession(result.data.totpVerifyStepUp.token, result.data.totpVerifyStepUp.user);
      setTotpActionCode("");
      setGlobalStatus(t("profile.totpStepUpVerified"));
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("profile.totpOperationFailed"));
    } finally {
      setTotpBusy(false);
    }
  }, [adoptSession, client, setGlobalStatus, t, totpActionCode]);

  const handleRegenerateTotpRecoveryCodes = useCallback(async () => {
    if (!totpActionCode.trim()) return;

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
      setGlobalStatus(t("profile.linkedAccountUnlinked"));
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("profile.linkedAccountUnlinkFailed"),
      );
    } finally {
      setUnlinkingAccountId(null);
    }
  }, [client, setGlobalStatus, t]);

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
      loadingPasskeys={loadingPasskeys}
      loadingTotp={loadingTotp}
      loadingLinkedAccounts={loadingLinkedAccounts}
      addingPasskey={addingPasskey}
      totpBusy={totpBusy}
      deletingPasskeyId={deletingPasskeyId}
      unlinkingAccountId={unlinkingAccountId}
      onAddPasskey={handleAddPasskey}
      onDeletePasskey={handleDeletePasskey}
      onStartTotpEnrollment={handleStartTotpEnrollment}
      onTotpEnrollmentCodeChange={setTotpEnrollmentCode}
      onCompleteTotpEnrollment={handleCompleteTotpEnrollment}
      onTotpActionCodeChange={setTotpActionCode}
      onVerifyTotpStepUp={handleVerifyTotpStepUp}
      onDisableTotp={handleDisableTotp}
      onRegenerateTotpRecoveryCodes={handleRegenerateTotpRecoveryCodes}
      onUnlinkExternalAccount={handleUnlinkExternalAccount}
    />
  );
}
