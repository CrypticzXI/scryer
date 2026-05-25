import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Separator } from "@/components/ui/separator";
import { Loader2 } from "lucide-react";
import { useTranslate } from "@/lib/context/translate-context";
import type { LinkedAccount, PasskeySummary } from "@/lib/types/settings";
import { selectorId } from "@/lib/utils/dom-ids";

type Props = {
  username?: string;
  currentPassword: string;
  newPassword: string;
  confirmPassword: string;
  saving: boolean;
  onCurrentPasswordChange: (value: string) => void;
  onNewPasswordChange: (value: string) => void;
  onConfirmPasswordChange: (value: string) => void;
  onChangePassword: () => void;
  showPasskeys: boolean;
  passkeys: PasskeySummary[];
  linkedAccounts: LinkedAccount[];
  loadingPasskeys: boolean;
  loadingLinkedAccounts: boolean;
  addingPasskey: boolean;
  deletingPasskeyId: string | null;
  unlinkingAccountId: string | null;
  onAddPasskey: () => void;
  onDeletePasskey: (id: string) => void;
  onUnlinkExternalAccount: (id: string) => void;
};

function formatTimestamp(value: string | null | undefined): string {
  if (!value) {
    return "—";
  }

  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) {
    return value;
  }

  return parsed.toLocaleString();
}

function providerLabel(provider: LinkedAccount["provider"]): string {
  switch (provider) {
    case "plex":
      return "Plex";
    case "jellyfin":
      return "Jellyfin";
    default:
      return provider;
  }
}

export function SettingsProfileSection({
  username,
  currentPassword,
  newPassword,
  confirmPassword,
  saving,
  onCurrentPasswordChange,
  onNewPasswordChange,
  onConfirmPasswordChange,
  onChangePassword,
  showPasskeys,
  passkeys,
  linkedAccounts,
  loadingPasskeys,
  loadingLinkedAccounts,
  addingPasskey,
  deletingPasskeyId,
  unlinkingAccountId,
  onAddPasskey,
  onDeletePasskey,
  onUnlinkExternalAccount,
}: Props) {
  const t = useTranslate();
  const passwordMismatch = confirmPassword.length > 0 && newPassword !== confirmPassword;
  const canSubmit = currentPassword.length > 0 && newPassword.length > 0 && !passwordMismatch && !saving;

  return (
    <div id="settings-profile-section" className="space-y-6 text-sm">
      <div className="space-y-2">
        <h3 className="text-base font-medium">{t("profile.accountInfo")}</h3>
        <div className="flex items-center gap-2 text-muted-foreground">
          <span>{t("settings.username")}:</span>
          <span className="font-medium text-foreground">{username ?? "—"}</span>
        </div>
      </div>

      <Separator />

      <div className="space-y-4">
        <h3 className="text-base font-medium">{t("profile.changePassword")}</h3>
        <div className="grid max-w-sm gap-3">
          <div className="space-y-1.5">
            <Label htmlFor="current-password">{t("profile.currentPassword")}</Label>
            <Input
              id="current-password"
              type="password"
              autoComplete="current-password"
              value={currentPassword}
              onChange={(e) => onCurrentPasswordChange(e.target.value)}
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="new-password">{t("profile.newPassword")}</Label>
            <Input
              id="new-password"
              type="password"
              autoComplete="new-password"
              value={newPassword}
              onChange={(e) => onNewPasswordChange(e.target.value)}
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="confirm-password">{t("profile.confirmPassword")}</Label>
            <Input
              id="confirm-password"
              type="password"
              autoComplete="new-password"
              value={confirmPassword}
              onChange={(e) => onConfirmPasswordChange(e.target.value)}
            />
            {passwordMismatch ? (
              <p className="text-xs text-destructive">{t("profile.passwordMismatch")}</p>
            ) : null}
          </div>
          <Button
            id={selectorId("settings-profile-change-password")}
            onClick={onChangePassword}
            disabled={!canSubmit}
            className="w-fit"
          >
            {saving ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
            {t("profile.changePassword")}
          </Button>
        </div>
      </div>

      {showPasskeys ? (
        <>
          <Separator />
          <div className="space-y-4">
            <div className="flex items-center justify-between gap-3">
              <div className="space-y-1">
                <h3 className="text-base font-medium">{t("profile.passkeys")}</h3>
                <p className="text-sm text-muted-foreground">{t("profile.passkeysDescription")}</p>
              </div>
              <Button
                id={selectorId("settings-profile-add-passkey")}
                onClick={onAddPasskey}
                disabled={addingPasskey}
                className="w-fit"
              >
                {addingPasskey ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
                {addingPasskey ? t("profile.passkeyAdding") : t("profile.passkeyAdd")}
              </Button>
            </div>

            {loadingPasskeys ? (
              <div className="flex items-center gap-2 text-sm text-muted-foreground">
                <Loader2 className="h-4 w-4 animate-spin" />
                <span>{t("label.loading")}</span>
              </div>
            ) : passkeys.length === 0 ? (
              <p className="text-sm text-muted-foreground">{t("profile.passkeysEmpty")}</p>
            ) : (
              <div className="space-y-3">
                {passkeys.map((passkey) => (
                  <div
                    key={passkey.id}
                    className="flex flex-col gap-3 rounded-md border border-border bg-background/60 p-4 md:flex-row md:items-center md:justify-between"
                  >
                    <div className="space-y-1">
                      <div className="font-medium text-foreground">
                        {passkey.friendlyName || t("profile.passkeyLabel")}
                      </div>
                      <div className="text-sm text-muted-foreground">
                        {t("profile.passkeyCreatedAt")}: {formatTimestamp(passkey.createdAt)}
                      </div>
                      <div className="text-sm text-muted-foreground">
                        {t("profile.passkeyLastUsedAt")}:{" "}
                        {passkey.lastUsedAt
                          ? formatTimestamp(passkey.lastUsedAt)
                          : t("profile.passkeyNeverUsed")}
                      </div>
                    </div>
                    <Button
                      id={selectorId(`settings-profile-delete-passkey-${passkey.id}`)}
                      variant="outline"
                      onClick={() => onDeletePasskey(passkey.id)}
                      disabled={deletingPasskeyId === passkey.id}
                      className="w-fit"
                    >
                      {deletingPasskeyId === passkey.id ? (
                        <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                      ) : null}
                      {t("label.delete")}
                    </Button>
                  </div>
                ))}
              </div>
            )}
          </div>
        </>
      ) : null}

      <Separator />
      <div className="space-y-4">
        <div className="space-y-1">
          <h3 className="text-base font-medium">{t("profile.linkedAccounts")}</h3>
          <p className="text-sm text-muted-foreground">
            {t("profile.linkedAccountsDescription")}
          </p>
        </div>

        {loadingLinkedAccounts ? (
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            <span>{t("label.loading")}</span>
          </div>
        ) : linkedAccounts.length === 0 ? (
          <p className="text-sm text-muted-foreground">{t("profile.linkedAccountsEmpty")}</p>
        ) : (
          <div className="space-y-3">
            {linkedAccounts.map((account) => (
              <div
                key={account.id}
                className="flex flex-col gap-3 rounded-md border border-border bg-background/60 p-4 md:flex-row md:items-center md:justify-between"
              >
                <div className="space-y-1">
                  <div className="font-medium text-foreground">
                    {providerLabel(account.provider)} · {account.displayName || account.username}
                  </div>
                  <div className="text-sm text-muted-foreground">
                    {t("profile.linkedAccountConnection")}: {account.connectionId}
                  </div>
                  <div className="text-sm text-muted-foreground">
                    {t("profile.linkedAccountStatus")}: {account.status}
                  </div>
                </div>
                <Button
                  id={selectorId(`settings-profile-unlink-account-${account.id}`)}
                  variant="outline"
                  onClick={() => onUnlinkExternalAccount(account.id)}
                  disabled={unlinkingAccountId === account.id}
                  className="w-fit"
                >
                  {unlinkingAccountId === account.id ? (
                    <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  ) : null}
                  {t("profile.unlinkAccount")}
                </Button>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
