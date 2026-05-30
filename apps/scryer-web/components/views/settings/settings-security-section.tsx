import type * as React from "react";
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
  confirmBusy: boolean;
  confirmUsername: string;
  confirmPassword: string;
  confirmError: string | null;
  onToggle: (enabled: boolean) => void;
  onConfirmUsernameChange: (value: string) => void;
  onConfirmPasswordChange: (value: string) => void;
  onConfirmEnable: () => Promise<void> | void;
  onCancelEnable: () => void;
  onConfirmDisable: () => Promise<void> | void;
  onCancelDisable: () => void;
  onSkipLocalIpsChange: (enabled: boolean) => void;
  onTotpConfigStepUpChange: (enabled: boolean) => void;
  onTotpJellyfinLoginChange: (enabled: boolean) => void;
  externalAccountInvitesPanel: React.ReactNode;
};

export function SettingsSecuritySection({
  settings,
  loading,
  enableConfirmOpen,
  disableConfirmOpen,
  confirmBusy,
  confirmUsername,
  confirmPassword,
  confirmError,
  onToggle,
  onConfirmUsernameChange,
  onConfirmPasswordChange,
  onConfirmEnable,
  onCancelEnable,
  onConfirmDisable,
  onCancelDisable,
  onSkipLocalIpsChange,
  onTotpConfigStepUpChange,
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
              <div className="grid gap-3 md:grid-cols-2">
                <div className="flex max-w-full items-start gap-3 rounded-md border border-border/70 bg-background/40 px-3 py-2">
                  <Checkbox
                    checked={settings.totpRequireConfigStepUp}
                    disabled={busy}
                    id="security-totp-config-step-up"
                    onCheckedChange={(checked) => onTotpConfigStepUpChange(checked === true)}
                  />
                  <div className="grid gap-1">
                    <div className="flex items-center gap-2">
                      <Label
                        className="cursor-pointer text-sm font-medium"
                        htmlFor="security-totp-config-step-up"
                      >
                        {t("settings.securityTotpConfigStepUp")}
                      </Label>
                      <InfoHelp
                        ariaLabel={t("settings.securityTotpConfigStepUp")}
                        text={t("settings.securityTotpConfigStepUpHelp")}
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
