import { Check, Download, Loader2, X } from "lucide-react";
import { DownloadClientRemotePathMappingsField } from "@/components/common/download-client-remote-path-mappings-field";
import { Button } from "@/components/ui/button";
import { SetupPanel, SetupStepHeader, SETUP_PRIMARY_CTA } from "./setup-chrome";
import { Input, integerInputProps, sanitizeDigits } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Checkbox } from "@/components/ui/checkbox";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import type { DownloadClientDraft, DownloadClientTypeOption } from "@/lib/types/download-clients";
import { buildWeaverApiKeyUrl } from "@/lib/utils/download-clients";
import type { LocalPathStyle } from "@/lib/utils/local-path-style";
import * as React from "react";

const DOWNLOAD_CLIENT_TYPE_ICON_SRC_BY_VALUE: Record<string, string> = {
  nzbget: "/download-clients/nzbget.svg",
  sabnzbd: "/download-clients/sabnzbd.svg",
  weaver: "/download-clients/weaver.webp",
  qbittorrent: "/download-clients/qbittorrent.svg",
};

function DownloadClientTypeOptionContent({
  typeValue,
  label,
}: {
  typeValue: string;
  label: string;
}) {
  const normalizedTypeValue = typeValue.trim().toLowerCase();
  const iconSrc = DOWNLOAD_CLIENT_TYPE_ICON_SRC_BY_VALUE[normalizedTypeValue];

  return (
    <span className="inline-flex min-w-0 items-center gap-2">
      {iconSrc ? (
        <img
          src={iconSrc}
          alt=""
          aria-hidden="true"
          className="h-4 w-4 shrink-0 object-contain"
        />
      ) : null}
      <span className="truncate">{label}</span>
    </span>
  );
}

interface SetupDownloadClientViewProps {
  t: (key: string) => string;
  draft: DownloadClientDraft;
  downloadClientTypeOptions: DownloadClientTypeOption[];
  localPathStyle: LocalPathStyle | undefined;
  onDraftChange: (updates: Partial<DownloadClientDraft>) => void;
  onTestConnection: () => void;
  onNext: () => void;
  onBack: () => void;
  onSkip?: () => void;
  testing: boolean;
  testResult: "success" | "failed" | null;
  saving: boolean;
  saved: boolean;
  error: string | null;
}

export function SetupDownloadClientView({
  t,
  draft,
  downloadClientTypeOptions,
  localPathStyle,
  onDraftChange,
  onTestConnection,
  onNext,
  onBack,
  onSkip,
  testing,
  testResult,
  saving,
  saved,
  error,
}: SetupDownloadClientViewProps) {
  const [areRemotePathMappingsValid, setAreRemotePathMappingsValid] = React.useState(true);
  const [isFilesystemPathMappingOpen, setIsFilesystemPathMappingOpen] = React.useState(() =>
    draft.remotePathMappings.trim().length > 0,
  );
  const showApiKey = draft.clientType === "sabnzbd" || draft.clientType === "weaver";
  const showCredentials =
    draft.clientType === "nzbget" ||
    draft.clientType === "qbittorrent" ||
    draft.clientType === "sabnzbd";
  const showSabAlternativeAuth = draft.clientType === "sabnzbd";
  const showDecypharrFilesystemHelp =
    draft.clientType === "sabnzbd" || draft.clientType === "qbittorrent";
  const weaverApiKeyUrl = draft.clientType === "weaver" ? buildWeaverApiKeyUrl(draft) : "";
  const normalizedClientType = draft.clientType.trim().toLowerCase();
  const selectedDownloadClientLabel =
    downloadClientTypeOptions.find((option) => option.value === normalizedClientType)?.label ??
    (draft.clientType.trim() || "Download client");
  React.useEffect(() => {
    if (draft.remotePathMappings.trim().length > 0 || !areRemotePathMappingsValid) {
      setIsFilesystemPathMappingOpen(true);
    } else {
      setIsFilesystemPathMappingOpen(false);
    }
  }, [areRemotePathMappingsValid, draft.remotePathMappings]);

  const canTest =
    draft.name.trim().length > 0 &&
    draft.host.trim().length > 0 &&
    areRemotePathMappingsValid;
  const canProceed = saved && areRemotePathMappingsValid;

  return (
    <SetupPanel id="setup-download-client-view" className="flex flex-col gap-6">
      <SetupStepHeader
        icon={Download}
        title={t("setup.downloadClientTitle")}
        subtitle={t("setup.downloadClientDescription")}
      />
      <div className="mx-auto flex w-full max-w-md flex-col gap-4">
        <div className="space-y-2">
          <Label htmlFor="setup-download-client-name">{t("label.name")}</Label>
          <Input
            id="setup-download-client-name"
            value={draft.name}
            onChange={(e) => onDraftChange({ name: e.target.value })}
            placeholder="My Download Client"
          />
        </div>
        <div className="space-y-2">
          <Label htmlFor="setup-download-client-type">{t("label.type")}</Label>
          <Select value={draft.clientType} onValueChange={(v) => onDraftChange({ clientType: v })}>
            <SelectTrigger id="setup-download-client-type">
              <SelectValue aria-label={selectedDownloadClientLabel}>
                <DownloadClientTypeOptionContent
                  typeValue={draft.clientType}
                  label={selectedDownloadClientLabel}
                />
              </SelectValue>
            </SelectTrigger>
            <SelectContent>
              {downloadClientTypeOptions.map((option) => (
                <SelectItem
                  key={option.value}
                  value={option.value}
                  textValue={option.label}
                >
                  <DownloadClientTypeOptionContent
                    typeValue={option.value}
                    label={option.label}
                  />
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="grid grid-cols-[1fr_auto] gap-2">
          <div className="space-y-2">
            <Label htmlFor="setup-download-client-host">{t("settings.host")}</Label>
            <Input
              id="setup-download-client-host"
              value={draft.host}
              onChange={(e) => onDraftChange({ host: e.target.value })}
              placeholder="192.168.1.100"
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="setup-download-client-port">{t("settings.port")}</Label>
            <Input
              id="setup-download-client-port"
              {...integerInputProps}
              className="w-24"
              value={draft.port}
              onChange={(e) => onDraftChange({ port: sanitizeDigits(e.target.value) })}
              placeholder="8080"
            />
          </div>
        </div>
        <div className="flex items-center gap-2">
          <Checkbox
            id="setup-download-client-ssl"
            checked={draft.useSsl}
            onCheckedChange={(checked) => onDraftChange({ useSsl: checked === true })}
          />
          <Label htmlFor="setup-download-client-ssl" className="text-sm">SSL</Label>
        </div>
        {showApiKey && (
          <div className="space-y-2">
            <Label htmlFor="setup-download-client-api-key">{t("settings.apiKey")}</Label>
            <Input
              id="setup-download-client-api-key"
              type="password"
              value={draft.apiKey}
              onChange={(e) => onDraftChange({ apiKey: e.target.value })}
            />
            {draft.clientType === "weaver" ? (
              <p className="text-xs text-muted-foreground">
                Create an integration API key in Weaver:{" "}
                {weaverApiKeyUrl ? (
                  <a
                    href={weaverApiKeyUrl}
                    target="_blank"
                    rel="noreferrer"
                    className="underline underline-offset-4 hover:text-foreground"
                  >
                    open Weaver security settings
                  </a>
                ) : (
                  <span>finish the Weaver URL above to generate the link.</span>
                )}
              </p>
            ) : draft.clientType === "sabnzbd" ? (
              <div className="space-y-2 text-xs text-muted-foreground">
                <p>{t("settings.downloadClientSabnzbdAuthHelp")}</p>
                <p>{t("settings.downloadClientSabnzbdNzbdavHelp")}</p>
              </div>
            ) : null}
          </div>
        )}
        {showCredentials && (
          <>
            <div className="space-y-2">
              <Label htmlFor="setup-download-client-username">
                {t("settings.username")}
                {showSabAlternativeAuth ? " (optional)" : ""}
              </Label>
              <Input
                id="setup-download-client-username"
                value={draft.username}
                onChange={(e) => onDraftChange({ username: e.target.value })}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="setup-download-client-password">
                {t("settings.password")}
                {showSabAlternativeAuth ? " (optional)" : ""}
              </Label>
              <Input
                id="setup-download-client-password"
                type="password"
                value={draft.password}
                onChange={(e) => onDraftChange({ password: e.target.value })}
              />
            </div>
            {draft.clientType === "qbittorrent" ? (
              <p className="text-xs text-muted-foreground">
                {t("settings.downloadClientQbittorrentDecypharrHelp")}
              </p>
            ) : null}
          </>
        )}
        {showDecypharrFilesystemHelp ? (
          <p className="text-xs text-muted-foreground">
            {t("settings.downloadClientDecypharrFilesystemHelp")}
          </p>
        ) : null}
        <details
          id="setup-download-client-filesystem-path-mapping"
          className="rounded-xl border border-border bg-card p-3"
          open={isFilesystemPathMappingOpen}
          onToggle={(event) =>
            setIsFilesystemPathMappingOpen(event.currentTarget.open)
          }
        >
          <summary
            id="setup-download-client-filesystem-path-mapping-toggle"
            className="cursor-pointer select-none text-sm font-medium text-card-foreground"
          >
            {t("settings.downloadClientFilesystemPathMapping")}
          </summary>
          <div className="mt-3 space-y-3">
            <p className="text-xs text-muted-foreground">
              {t("settings.downloadClientFilesystemPathMappingHelp")}
            </p>
            <DownloadClientRemotePathMappingsField
              fieldKey="remote_path_mappings"
              label={t("settings.downloadClientRemotePathMappings")}
              value={draft.remotePathMappings}
              helpText={t("settings.downloadClientRemotePathMappingsHelp")}
              localPathStyle={localPathStyle}
              translate={t}
              onValidityChange={setAreRemotePathMappingsValid}
              onChange={(_, value) => onDraftChange({ remotePathMappings: value })}
            />
          </div>
        </details>
        <div className="flex items-center gap-3">
          <Button
            id="setup-download-client-test-connection"
            variant="outline"
            onClick={onTestConnection}
            disabled={!canTest || testing || saving}
          >
            {testing ? (
              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
            ) : null}
            {t("label.testConnection")}
          </Button>
          {testResult === "success" && (
            <span className="flex items-center gap-1 text-sm text-emerald-500">
              <Check className="h-4 w-4" /> {t("setup.connectionSuccess")}
            </span>
          )}
          {testResult === "failed" && (
            <span className="flex items-center gap-1 text-sm text-destructive">
              <X className="h-4 w-4" /> {t("setup.connectionFailed")}
            </span>
          )}
        </div>
        {error && <p id="setup-download-client-error" className="text-sm text-destructive">{error}</p>}
        {saved && (
          <p id="setup-download-client-saved" className="text-sm text-emerald-500">{t("setup.saved")}</p>
        )}
      </div>
      <div className="flex items-center justify-between pt-2">
        <Button id="setup-download-client-back" variant="ghost" onClick={onBack}>{t("setup.back")}</Button>
        <div className="flex items-center gap-3">
          {onSkip && (
            <Button id="setup-download-client-skip" type="button" variant="link" onClick={onSkip}>
              {t("setup.skip")}
            </Button>
          )}
          <Button id="setup-download-client-next" className={SETUP_PRIMARY_CTA} onClick={onNext} disabled={!canProceed || saving}>
            {saving ? t("label.saving") : t("setup.next")}
          </Button>
        </div>
      </div>
    </SetupPanel>
  );
}
