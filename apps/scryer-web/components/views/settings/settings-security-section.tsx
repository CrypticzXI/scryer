import type * as React from "react";
import { InfoHelp } from "@/components/common/info-help";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Loader2, Plus, Trash2 } from "lucide-react";
import { useTranslate } from "@/lib/context/translate-context";
import type {
  AuthProviderConnection,
  AuthProviderSettings,
  ExternalAccountProvider,
  SecuritySettings,
} from "@/lib/types/settings";

const AUTH_PROVIDERS: ExternalAccountProvider[] = ["plex", "jellyfin"];

type SettingsSecuritySectionProps = {
  settings: SecuritySettings;
  authProviderSettings: AuthProviderSettings;
  loading: boolean;
  authProviderSaving: boolean;
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
  onAuthProviderSettingsChange: (settings: AuthProviderSettings) => void;
  onAuthProviderSettingsSave: () => Promise<void> | void;
  externalAccountInvitesPanel: React.ReactNode;
};

function providerLabel(provider: ExternalAccountProvider): string {
  switch (provider) {
    case "plex":
      return "Plex";
    case "jellyfin":
      return "Jellyfin";
    default:
      return provider;
  }
}

function toggleProviderList(
  current: ExternalAccountProvider[],
  provider: ExternalAccountProvider,
  enabled: boolean,
): ExternalAccountProvider[] {
  const next = new Set(current);
  if (enabled) {
    next.add(provider);
  } else {
    next.delete(provider);
  }
  return AUTH_PROVIDERS.filter((candidate) => next.has(candidate));
}

function connectionIds(connections: AuthProviderConnection[]): string[] {
  return Array.from(
    new Set(connections.map((connection) => connection.id.trim()).filter(Boolean)),
  );
}

function emptyConnection(): AuthProviderConnection {
  return {
    id: "",
    displayName: "",
    userVisibleUrl: null,
    baseUrl: null,
    machineId: null,
  };
}

export function SettingsSecuritySection({
  settings,
  authProviderSettings,
  loading,
  authProviderSaving,
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
  onAuthProviderSettingsChange,
  onAuthProviderSettingsSave,
  externalAccountInvitesPanel,
}: SettingsSecuritySectionProps) {
  const t = useTranslate();
  const busy = loading || confirmBusy || authProviderSaving;
  const confirmDisabled =
    confirmUsername.trim().length === 0 || confirmPassword.trim().length === 0;

  const updateProviderList = (
    key: "allowedProviders" | "providerLoginEnabled" | "providerLinkingEnabled",
    provider: ExternalAccountProvider,
    enabled: boolean,
  ) => {
    onAuthProviderSettingsChange({
      ...authProviderSettings,
      [key]: toggleProviderList(authProviderSettings[key], provider, enabled),
    });
  };

  const updateConnections = (
    key: "allowedJellyfinConnections" | "allowedPlexConnections",
    connections: AuthProviderConnection[],
  ) => {
    const idKey =
      key === "allowedJellyfinConnections"
        ? "allowedJellyfinConnectionIds"
        : "allowedPlexConnectionIds";
    onAuthProviderSettingsChange({
      ...authProviderSettings,
      [key]: connections,
      [idKey]: connectionIds(connections),
    });
  };

  const updateConnection = (
    key: "allowedJellyfinConnections" | "allowedPlexConnections",
    index: number,
    patch: Partial<AuthProviderConnection>,
  ) => {
    updateConnections(
      key,
      authProviderSettings[key].map((connection, candidateIndex) =>
        candidateIndex === index ? { ...connection, ...patch } : connection,
      ),
    );
  };

  const addConnection = (key: "allowedJellyfinConnections" | "allowedPlexConnections") => {
    updateConnections(key, [...authProviderSettings[key], emptyConnection()]);
  };

  const removeConnection = (
    key: "allowedJellyfinConnections" | "allowedPlexConnections",
    index: number,
  ) => {
    updateConnections(
      key,
      authProviderSettings[key].filter((_, candidateIndex) => candidateIndex !== index),
    );
  };

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

        <div className="rounded-lg border border-border bg-card/50 p-4">
          <div className="space-y-4">
            <div className="space-y-1">
              <div className="flex items-center gap-2">
                <Label className="text-sm font-medium">
                  {t("settings.authProvidersTitle")}
                </Label>
                {authProviderSaving ? (
                  <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
                ) : null}
              </div>
              <p className="text-xs text-muted-foreground">
                {t("settings.authProvidersDescription")}
              </p>
            </div>

            <div className="grid gap-3 lg:grid-cols-3">
              {AUTH_PROVIDERS.map((provider) => (
                <div
                  key={provider}
                  className="space-y-3 rounded-md border border-border/70 bg-background/40 p-3"
                >
                  <div className="font-medium">{providerLabel(provider)}</div>
                  <label className="flex items-center gap-2">
                    <Checkbox
                      checked={authProviderSettings.allowedProviders.includes(provider)}
                      disabled={busy}
                      onCheckedChange={(checked) =>
                        updateProviderList("allowedProviders", provider, checked === true)
                      }
                    />
                    <span>{t("settings.authProviderAllowed")}</span>
                  </label>
                  <label className="flex items-center gap-2">
                    <Checkbox
                      checked={authProviderSettings.providerLoginEnabled.includes(provider)}
                      disabled={busy}
                      onCheckedChange={(checked) =>
                        updateProviderList("providerLoginEnabled", provider, checked === true)
                      }
                    />
                    <span>{t("settings.authProviderLoginEnabled")}</span>
                  </label>
                  <label className="flex items-center gap-2">
                    <Checkbox
                      checked={authProviderSettings.providerLinkingEnabled.includes(provider)}
                      disabled={busy}
                      onCheckedChange={(checked) =>
                        updateProviderList("providerLinkingEnabled", provider, checked === true)
                      }
                    />
                    <span>{t("settings.authProviderLinkingEnabled")}</span>
                  </label>
                </div>
              ))}
            </div>

            <div className="grid gap-4 md:grid-cols-2">
              {[
                {
                  key: "allowedJellyfinConnections" as const,
                  title: t("settings.allowedJellyfinConnections"),
                  idPrefix: "jellyfin",
                  servicePlaceholder: "jellyfin-main",
                  namePlaceholder: "Home Jellyfin",
                  endpointLabel: t("settings.connectionBaseUrl"),
                  endpointPlaceholder: "https://jellyfin.example.test",
                  endpointField: "baseUrl" as const,
                },
                {
                  key: "allowedPlexConnections" as const,
                  title: t("settings.allowedPlexConnections"),
                  idPrefix: "plex",
                  servicePlaceholder: "plex-main",
                  namePlaceholder: "Home Plex",
                  endpointLabel: t("settings.plexMachineId"),
                  endpointPlaceholder: "optional machineIdentifier",
                  endpointField: "machineId" as const,
                },
              ].map((group) => (
                <div key={group.key} className="space-y-3">
                  <div className="flex items-center justify-between gap-3">
                    <Label>{group.title}</Label>
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      disabled={busy}
                      onClick={() => addConnection(group.key)}
                    >
                      <Plus className="h-4 w-4" />
                      {t("settings.addConnection")}
                    </Button>
                  </div>
                  <div className="space-y-3">
                    {authProviderSettings[group.key].length === 0 ? (
                      <div className="rounded-md border border-dashed border-border/70 px-3 py-3 text-xs text-muted-foreground">
                        {t("settings.noConnectionsConfigured")}
                      </div>
                    ) : null}
                    {authProviderSettings[group.key].map((connection, index) => (
                      <div
                        key={`${group.key}-${index}`}
                        className="space-y-3 rounded-md border border-border/70 bg-background/40 p-3"
                      >
                        <div className="grid gap-3 sm:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_auto]">
                          <div className="space-y-1.5">
                            <Label htmlFor={`${group.idPrefix}-connection-id-${index}`}>
                              {t("settings.connectionId")}
                            </Label>
                            <Input
                              id={`${group.idPrefix}-connection-id-${index}`}
                              value={connection.id}
                              disabled={busy}
                              placeholder={group.servicePlaceholder}
                              onChange={(event) =>
                                updateConnection(group.key, index, {
                                  id: event.target.value,
                                })
                              }
                            />
                          </div>
                          <div className="space-y-1.5">
                            <Label htmlFor={`${group.idPrefix}-connection-name-${index}`}>
                              {t("settings.connectionName")}
                            </Label>
                            <Input
                              id={`${group.idPrefix}-connection-name-${index}`}
                              value={connection.displayName}
                              disabled={busy}
                              placeholder={group.namePlaceholder}
                              onChange={(event) =>
                                updateConnection(group.key, index, {
                                  displayName: event.target.value,
                                })
                              }
                            />
                          </div>
                          <div className="flex items-end">
                            <Button
                              type="button"
                              variant="ghost"
                              size="icon-sm"
                              disabled={busy}
                              aria-label={t("settings.removeConnection")}
                              title={t("settings.removeConnection")}
                              onClick={() => removeConnection(group.key, index)}
                            >
                              <Trash2 className="h-4 w-4" />
                            </Button>
                          </div>
                        </div>
                        <div className="space-y-1.5">
                          <Label htmlFor={`${group.idPrefix}-connection-endpoint-${index}`}>
                            {group.endpointLabel}
                          </Label>
                          <Input
                            id={`${group.idPrefix}-connection-endpoint-${index}`}
                            value={connection[group.endpointField] ?? ""}
                            disabled={busy}
                            placeholder={group.endpointPlaceholder}
                            onChange={(event) =>
                              updateConnection(group.key, index, {
                                [group.endpointField]: event.target.value,
                              })
                            }
                          />
                        </div>
                      </div>
                    ))}
                  </div>
                </div>
              ))}
            </div>

            <Button
              id="settings-auth-provider-save"
              type="button"
              className="w-fit"
              disabled={busy}
              onClick={onAuthProviderSettingsSave}
            >
              {authProviderSaving ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
              {authProviderSaving ? t("label.saving") : t("label.save")}
            </Button>
          </div>
        </div>
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
