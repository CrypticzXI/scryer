import type * as React from "react";
import { Link } from "react-router-dom";
import { InfoHelp } from "@/components/common/info-help";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Loader2 } from "lucide-react";
import { useTranslate } from "@/lib/context/translate-context";
import type { SecuritySettings } from "@/lib/types/settings";

type SettingsSecuritySectionProps = {
  settings: SecuritySettings;
  loading: boolean;
  enableConfirmOpen: boolean;
  disableConfirmOpen: boolean;
  adminPasswordRequiredOpen: boolean;
  confirmBusy: boolean;
  confirmUsername: string;
  confirmPassword: string;
  confirmError: string | null;
  passwordMinLengthDraft: string;
  minPasswordLength: number;
  onToggle: (enabled: boolean) => void;
  onConfirmUsernameChange: (value: string) => void;
  onConfirmPasswordChange: (value: string) => void;
  onConfirmEnable: () => Promise<void> | void;
  onCancelEnable: () => void;
  onConfirmDisable: () => Promise<void> | void;
  onCancelDisable: () => void;
  onConfirmAdminPasswordRequired: () => void;
  onCancelAdminPasswordRequired: () => void;
  onPasswordMinLengthDraftChange: (value: string) => void;
  onPasswordMinLengthSubmit: (value?: string) => Promise<void> | void;
  onSkipLocalIpsChange: (enabled: boolean) => void;
  onMfaConfigStepUpChange: (enabled: boolean) => void;
  onMfaPasswordLoginChange: (enabled: boolean) => void;
  onTotpJellyfinLoginChange: (enabled: boolean) => void;
  externalAccountInvitesPanel: React.ReactNode;
};

export function SettingsSecuritySection({
  settings,
  loading,
  enableConfirmOpen,
  disableConfirmOpen,
  adminPasswordRequiredOpen,
  confirmBusy,
  confirmUsername,
  confirmPassword,
  confirmError,
  passwordMinLengthDraft,
  minPasswordLength,
  onToggle,
  onConfirmUsernameChange,
  onConfirmPasswordChange,
  onConfirmEnable,
  onCancelEnable,
  onConfirmDisable,
  onCancelDisable,
  onConfirmAdminPasswordRequired,
  onCancelAdminPasswordRequired,
  onPasswordMinLengthDraftChange,
  onPasswordMinLengthSubmit,
  onSkipLocalIpsChange,
  onMfaConfigStepUpChange,
  onMfaPasswordLoginChange,
  onTotpJellyfinLoginChange,
  externalAccountInvitesPanel,
}: SettingsSecuritySectionProps) {
  const t = useTranslate();
  const busy = loading || confirmBusy;
  const confirmDisabled =
    confirmUsername.trim().length === 0 || confirmPassword.trim().length === 0;

  return (
    <>
      <div id="settings-security-section" className="space-y-6 text-sm">
        <div className="space-y-2">
          <h3 className="text-base font-medium">{t("settings.security")}</h3>
          <p className="max-w-2xl text-muted-foreground">
            {t("settings.securityDescription")}
          </p>
        </div>

        <div className="rounded-lg border border-border bg-card/50 p-4">
          <div className="space-y-4">
            <div className="space-y-1">
              <div className="flex items-center gap-2">
                <Label className="text-sm font-medium">
                  {t("settings.securityEnableFormLogin")}
                </Label>
                {busy ? <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" /> : null}
              </div>
              <p className="text-xs text-muted-foreground">
                {t("settings.securityEnableFormLoginHelp")}
              </p>
            </div>
            <div className="max-w-xs space-y-1.5">
              <Label className="text-sm font-medium" htmlFor="security-password-min-length">
                {t("settings.securityPasswordMinLength")}
              </Label>
              <Input
                id="security-password-min-length"
                type="number"
                inputMode="numeric"
                min={minPasswordLength}
                step={1}
                value={passwordMinLengthDraft}
                disabled={busy}
                onBlur={(event) =>
                  void onPasswordMinLengthSubmit(event.currentTarget.value)
                }
                onChange={(event) => onPasswordMinLengthDraftChange(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    event.preventDefault();
                    void onPasswordMinLengthSubmit(event.currentTarget.value);
                  }
                }}
              />
              <p className="text-xs text-muted-foreground">
                {t("settings.securityPasswordMinLengthHelp", {
                  min: minPasswordLength,
                })}
              </p>
            </div>
            <div className="space-y-3">
              <Button
                id="settings-security-toggle-form-login"
                type="button"
                aria-pressed={settings.formLoginEnabled}
                variant={settings.formLoginEnabled ? "destructive" : "primary"}
                disabled={busy}
                onClick={() => onToggle(!settings.formLoginEnabled)}
              >
                {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                {settings.formLoginEnabled ? t("label.disable") : t("label.enable")}
              </Button>
              <div className="flex w-fit max-w-full items-center gap-3 rounded-md border border-border/70 bg-background/40 px-3 py-2">
                <Checkbox
                  checked={settings.skipLoginForLocalIps}
                  disabled={busy}
                  id="security-skip-local-ips"
                  onCheckedChange={(checked) => onSkipLocalIpsChange(checked === true)}
                />
                <div className="flex items-center gap-2">
                  <Label
                    className="cursor-pointer text-sm font-medium"
                    htmlFor="security-skip-local-ips"
                  >
                    {t("settings.securitySkipLocalIps")}
                  </Label>
                  <InfoHelp
                    ariaLabel={t("settings.securitySkipLocalIps")}
                    text={t("settings.securitySkipLocalIpsHelp")}
                  />
                </div>
              </div>
              <div className="grid gap-3 md:grid-cols-3">
                <div className="flex max-w-full items-start gap-3 rounded-md border border-border/70 bg-background/40 px-3 py-2">
                  <Checkbox
                    checked={settings.mfaRequireConfigStepUp}
                    disabled={busy}
                    id="security-mfa-config-step-up"
                    onCheckedChange={(checked) => onMfaConfigStepUpChange(checked === true)}
                  />
                  <div className="grid gap-1">
                    <div className="flex items-center gap-2">
                      <Label
                        className="cursor-pointer text-sm font-medium"
                        htmlFor="security-mfa-config-step-up"
                      >
                        {t("settings.securityMfaConfigStepUp")}
                      </Label>
                      <InfoHelp
                        ariaLabel={t("settings.securityMfaConfigStepUp")}
                        text={t("settings.securityMfaConfigStepUpHelp")}
                      />
                    </div>
                  </div>
                </div>
                <div className="flex max-w-full items-start gap-3 rounded-md border border-border/70 bg-background/40 px-3 py-2">
                  <Checkbox
                    checked={settings.mfaRequirePasswordLogin}
                    disabled={busy}
                    id="security-mfa-password-login"
                    onCheckedChange={(checked) => onMfaPasswordLoginChange(checked === true)}
                  />
                  <div className="grid gap-1">
                    <div className="flex items-center gap-2">
                      <Label
                        className="cursor-pointer text-sm font-medium"
                        htmlFor="security-mfa-password-login"
                      >
                        {t("settings.securityMfaPasswordLogin")}
                      </Label>
                      <InfoHelp
                        ariaLabel={t("settings.securityMfaPasswordLogin")}
                        text={t("settings.securityMfaPasswordLoginHelp")}
                      />
                    </div>
                  </div>
                </div>
                <div className="flex max-w-full items-start gap-3 rounded-md border border-border/70 bg-background/40 px-3 py-2">
                  <Checkbox
                    checked={settings.totpRequireJellyfinLogin}
                    disabled={busy}
                    id="security-totp-jellyfin-login"
                    onCheckedChange={(checked) => onTotpJellyfinLoginChange(checked === true)}
                  />
                  <div className="grid gap-1">
                    <div className="flex items-center gap-2">
                      <Label
                        className="cursor-pointer text-sm font-medium"
                        htmlFor="security-totp-jellyfin-login"
                      >
                        {t("settings.securityTotpJellyfinLogin")}
                      </Label>
                      <InfoHelp
                        ariaLabel={t("settings.securityTotpJellyfinLogin")}
                        text={t("settings.securityTotpJellyfinLoginHelp")}
                      />
                    </div>
                  </div>
                </div>
              </div>
            </div>
          </div>
        </div>

        <div className="rounded-lg border border-border bg-card/50 p-4">
          <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
            <div className="space-y-1">
              <h4 className="text-sm font-medium">
                {t("settings.manageMediaServerLogins")}
              </h4>
              <p className="max-w-2xl text-xs text-muted-foreground">
                {t("settings.manageMediaServerLoginsDescription")}
              </p>
            </div>
            <Button asChild variant="outline" className="w-fit shrink-0">
              <Link to="/settings/media-servers">
                {t("settings.openMediaServers")}
              </Link>
            </Button>
          </div>
        </div>

        {settings.envOverrideActive ? (
          <div className="space-y-3 rounded-lg border border-amber-500/30 bg-amber-500/10 p-4">
            <div className="space-y-1">
              <h4 className="text-sm font-medium">{t("settings.securityOverrideTitle")}</h4>
              <p className="text-xs text-muted-foreground">
                {t("settings.securityOverrideDescription")}
              </p>
              {settings.envOverrideDescription ? (
                <p className="text-xs text-muted-foreground">
                  {t("settings.securityOverrideReason", {
                    override: settings.envOverrideDescription,
                  })}
                </p>
              ) : null}
            </div>
            <div className="grid gap-3 sm:grid-cols-2">
              <div className="rounded-md border border-border/70 bg-background/40 p-3">
                <div className="text-xs uppercase tracking-wide text-muted-foreground">
                  {t("settings.securitySavedPreference")}
                </div>
                <div className="mt-1 font-medium">
                  {settings.formLoginEnabled
                    ? t("settings.securityModeEnabled")
                    : t("settings.securityModeDisabled")}
                </div>
              </div>
              <div className="rounded-md border border-border/70 bg-background/40 p-3">
                <div className="text-xs uppercase tracking-wide text-muted-foreground">
                  {t("settings.securityEffectiveMode")}
                </div>
                <div className="mt-1 font-medium">
                  {settings.effectiveFormLoginEnabled
                    ? t("settings.securityModeEnabled")
                    : t("settings.securityModeDisabled")}
                </div>
              </div>
            </div>
          </div>
        ) : null}

      </div>

      {externalAccountInvitesPanel}

      <ConfirmDialog
        open={adminPasswordRequiredOpen}
        contentId="settings-security-admin-password-required-dialog"
        title={t("settings.securityAdminPasswordRequiredTitle")}
        description={t("settings.securityAdminPasswordRequiredDescription")}
        confirmLabel={t("settings.securityAdminPasswordRequiredAction")}
        cancelLabel={t("label.cancel")}
        confirmButtonId="settings-security-admin-password-required-confirm"
        cancelButtonId="settings-security-admin-password-required-cancel"
        confirmButtonVariant="default"
        onConfirm={onConfirmAdminPasswordRequired}
        onCancel={onCancelAdminPasswordRequired}
      />

      <ConfirmDialog
        open={enableConfirmOpen}
        contentId="settings-security-enable-dialog"
        title={t("settings.securityConfirmTitle")}
        description={t("settings.securityConfirmDescription")}
        confirmLabel={t("settings.securityConfirmAction")}
        cancelLabel={t("label.cancel")}
        confirmButtonId="settings-security-enable-confirm"
        cancelButtonId="settings-security-enable-cancel"
        confirmButtonVariant="default"
        confirmButtonClassName="bg-emerald-600 text-white hover:bg-emerald-700 focus-visible:ring-emerald-600 dark:bg-emerald-500 dark:text-emerald-950 dark:hover:bg-emerald-400"
        isBusy={confirmBusy}
        confirmDisabled={confirmDisabled}
        onConfirm={onConfirmEnable}
        onCancel={onCancelEnable}
      >
        <div className="space-y-3">
          <div className="space-y-1.5">
            <Label htmlFor="security-confirm-username">
              {t("settings.securityConfirmUsername")}
            </Label>
            <Input
              id="security-confirm-username"
              autoComplete="username"
              value={confirmUsername}
              onChange={(event) => onConfirmUsernameChange(event.target.value)}
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="security-confirm-password">
              {t("settings.securityConfirmPassword")}
            </Label>
            <Input
              id="security-confirm-password"
              type="password"
              autoComplete="current-password"
              value={confirmPassword}
              onChange={(event) => onConfirmPasswordChange(event.target.value)}
            />
          </div>
          {confirmError ? (
            <p id="settings-security-confirm-error" className="text-xs text-destructive">
              {confirmError}
            </p>
          ) : null}
        </div>
      </ConfirmDialog>

      <ConfirmDialog
        open={disableConfirmOpen}
        contentId="settings-security-disable-dialog"
        title={t("settings.securityDisableConfirmTitle")}
        description={t("settings.securityDisableConfirmDescription")}
        confirmLabel={t("settings.securityDisableConfirmAction")}
        cancelLabel={t("label.cancel")}
        confirmButtonId="settings-security-disable-confirm"
        cancelButtonId="settings-security-disable-cancel"
        isBusy={confirmBusy}
        onConfirm={onConfirmDisable}
        onCancel={onCancelDisable}
      />
    </>
  );
}
