import { useState, useCallback, useEffect } from "react";
import { useClient } from "urql";
import { SettingsProfileSection } from "@/components/views/settings/settings-profile-section";
import {
  deleteMyPasskeyMutation,
  setUserPasswordMutation,
  unlinkExternalAccountMutation,
} from "@/lib/graphql/mutations";
import { linkedAccountsQuery, myPasskeysQuery } from "@/lib/graphql/queries";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useAuth } from "@/lib/hooks/use-auth";
import type { LinkedAccount, PasskeySummary } from "@/lib/types/settings";
import { PasskeyClientError, registerPasskey } from "@/lib/utils/passkeys";

type Props = {
  userId?: string;
  username?: string;
};

export function SettingsProfileContainer({ userId, username }: Props) {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const { passkeyEnabled } = useAuth();
  const [currentPassword, setCurrentPassword] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [saving, setSaving] = useState(false);
  const [passkeys, setPasskeys] = useState<PasskeySummary[]>([]);
  const [linkedAccounts, setLinkedAccounts] = useState<LinkedAccount[]>([]);
  const [loadingPasskeys, setLoadingPasskeys] = useState(false);
  const [loadingLinkedAccounts, setLoadingLinkedAccounts] = useState(false);
  const [addingPasskey, setAddingPasskey] = useState(false);
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
            currentPassword,
          },
        })
        .toPromise();

      if (result.error) {
        setGlobalStatus(result.error.message);
        return;
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
  }, [client, userId, currentPassword, newPassword, confirmPassword, setGlobalStatus, t]);

  const loadPasskeys = useCallback(async () => {
    if (!passkeyEnabled || !userId) {
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
  }, [client, passkeyEnabled, setGlobalStatus, t, userId]);

  useEffect(() => {
    void loadPasskeys();
  }, [loadPasskeys]);

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
    if (!passkeyEnabled || !userId) return;

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
  }, [client, formatPasskeyError, passkeyEnabled, setGlobalStatus, t, userId]);

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
      onCurrentPasswordChange={setCurrentPassword}
      onNewPasswordChange={setNewPassword}
      onConfirmPasswordChange={setConfirmPassword}
      onChangePassword={handleChangePassword}
      showPasskeys={passkeyEnabled && Boolean(userId)}
      passkeys={passkeys}
      linkedAccounts={linkedAccounts}
      loadingPasskeys={loadingPasskeys}
      loadingLinkedAccounts={loadingLinkedAccounts}
      addingPasskey={addingPasskey}
      deletingPasskeyId={deletingPasskeyId}
      unlinkingAccountId={unlinkingAccountId}
      onAddPasskey={handleAddPasskey}
      onDeletePasskey={handleDeletePasskey}
      onUnlinkExternalAccount={handleUnlinkExternalAccount}
    />
  );
}
