import { useState, useMemo, useRef } from "react";
import { ArrowUpCircle, Download, ExternalLink, Loader2, Power, PowerOff, RefreshCw, Trash2, Upload } from "lucide-react";
import { RenderBooleanIcon } from "@/components/common/boolean-icon";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Progress } from "@/components/ui/progress";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useTranslate } from "@/lib/context/translate-context";
import { selectorId } from "@/lib/utils/dom-ids";
import { cn } from "@/lib/utils";
import {
  boxedActionButtonBaseClass,
  boxedActionButtonToneClass,
  type BoxedActionButtonTone,
} from "@/lib/utils/action-button-styles";

export type RegistryPluginRecord = {
  id: string;
  name: string;
  description: string;
  version: string;
  latestVersion?: string | null;
  pluginType: string;
  providerType: string;
  author: string;
  official: boolean;
  publisher?: string | null;
  supportTier?: string | null;
  docsUrl?: string | null;
  sourceRepo?: string | null;
  builtin: boolean;
  sourceUrl: string | null;
  sourceKind?: string | null;
  blockedReason?: string | null;
  bytes?: number | null;
  isInstalled: boolean;
  isEnabled: boolean;
  installedVersion: string | null;
  updateAvailable: boolean;
  installInProgress: boolean;
  defaultBaseUrl?: string | null;
};

export type PluginInstallProgressRecord = {
  pluginId: string;
  operationKind: "install" | "upgrade";
  state: "downloading" | "verifying" | "installing" | "succeeded" | "failed";
  label: string;
  stepIndex: number;
  stepCount: number;
  message?: string | null;
  error?: string | null;
};

export type PluginCatalogStatusRecord = {
  refreshState: string;
  githubAvailable: boolean;
  lastCheckedAt?: string | null;
  outageMessage?: string | null;
  blockedActions: string[];
  restoreWarnings: string[];
  lastError?: string | null;
};

export type ManualPluginPreviewRecord = {
  githubRepoUrl: string;
  plugin: RegistryPluginRecord;
};

type SettingsPluginsSectionProps = {
  plugins: RegistryPluginRecord[];
  catalogStatus: PluginCatalogStatusRecord | null;
  initialLoading: boolean;
  mutatingPluginIds: string[];
  pluginProgress: Partial<Record<string, PluginInstallProgressRecord>>;
  pluginErrors: Partial<Record<string, string>>;
  refreshing: boolean;
  upgradingAll: boolean;
  manualRepoUrl: string;
  manualFileName: string | null;
  manualPreview: ManualPluginPreviewRecord | null;
  manualBusy: boolean;
  showManualInstall: boolean;
  remoteActionsBlocked: {
    refresh: boolean;
    install: boolean;
    installManual: boolean;
    upgrade: boolean;
    inspectManual: boolean;
  };
  onManualRepoUrlChange: (value: string) => void;
  onToggleManualInstall: () => void;
  onManualPluginFileChange: (event: React.ChangeEvent<HTMLInputElement>) => void;
  onInspectManualPluginRepo: () => void;
  onRequestInstallUploadedPlugin: () => void;
  onInstallManualPlugin: () => void;
  onRefreshRegistry: () => void;
  onUpgradeAllPlugins: () => void;
  onTogglePlugin: (plugin: RegistryPluginRecord) => void;
  onInstallPlugin: (plugin: RegistryPluginRecord) => void;
  onUninstallPlugin: (plugin: RegistryPluginRecord) => void;
  onUpgradePlugin: (plugin: RegistryPluginRecord) => void;
};

type FilterState = {
  category: string;
  officialOnly: boolean;
};

type Translate = (key: string, values?: Record<string, string | number | boolean | null | undefined>) => string;

function isDownloadedBuiltinOverride(plugin: RegistryPluginRecord): boolean {
  return plugin.builtin && plugin.sourceKind === "downloaded";
}

function canUninstallPlugin(plugin: RegistryPluginRecord): boolean {
  return !plugin.builtin || plugin.sourceKind === "downloaded";
}

function uninstallLabel(plugin: RegistryPluginRecord, t: Translate): string {
  return isDownloadedBuiltinOverride(plugin)
    ? t("settings.pluginRevertToBundled")
    : t("settings.pluginUninstall");
}

function installIsBlocked(plugin: RegistryPluginRecord): boolean {
  return plugin.blockedReason === "no_compatible_release";
}

function blockedReasonLabel(plugin: RegistryPluginRecord, t: Translate): string | null {
  switch (plugin.blockedReason) {
    case "no_compatible_release":
      return t("settings.pluginNoCompatibleRelease");
    case "newer_release_requires_newer_scryer":
      return plugin.latestVersion
        ? t("settings.pluginNewerReleaseRequiresNewerScryerVersion", {
          version: plugin.latestVersion,
        })
        : t("settings.pluginNewerReleaseRequiresNewerScryer");
    default:
      return null;
  }
}

function formatPluginBytes(bytes?: number | null): string | null {
  if (bytes == null) {
    return null;
  }
  if (bytes < 1024) {
    return `${bytes} B`;
  }
  if (bytes < 1024 * 1024) {
    return `${(bytes / 1024).toFixed(1)} KB`;
  }
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function isRunningPluginProgress(
  progress?: PluginInstallProgressRecord,
): progress is PluginInstallProgressRecord {
  return progress !== undefined
    && progress.state !== "succeeded"
    && progress.state !== "failed";
}

function pluginProgressLabel(progress: PluginInstallProgressRecord, t: Translate): string {
  switch (progress.state) {
    case "downloading":
      return t("settings.pluginInstallDownloading");
    case "verifying":
      return t("settings.pluginInstallVerifying");
    case "installing":
      return t("settings.pluginInstallInstalling");
    case "succeeded":
    case "failed":
      return progress.label;
    default:
      return progress.label;
  }
}

export function PluginInstallProgressBar({
  progress,
  id,
  className = "space-y-1 overflow-hidden",
}: {
  progress: PluginInstallProgressRecord;
  id?: string;
  className?: string;
}) {
  const t = useTranslate();

  return (
    <div className={className}>
      <div className="truncate text-right text-xs leading-tight text-primary">
        {pluginProgressLabel(progress, t)}
      </div>
      <Progress
        id={id}
        value={(progress.stepIndex / Math.max(progress.stepCount, 1)) * 100}
        className="h-1.5"
      />
    </div>
  );
}

function normalizePluginLink(url?: string | null): string | null {
  if (!url) {
    return null;
  }
  const trimmed = url.trim();
  if (!trimmed) {
    return null;
  }
  return trimmed.replace(/\/+$/, "");
}

function PluginActionButton({
  label,
  tone,
  className,
  children,
  ...props
}: React.ComponentProps<typeof Button> & {
  label: string;
  tone: Extract<BoxedActionButtonTone, "install" | "upgrade" | "enabled" | "disabled" | "delete">;
}) {
  return (
    <Button
      type="button"
      size="icon-sm"
      variant="secondary"
      title={label}
      aria-label={label}
      className={cn(
        boxedActionButtonBaseClass,
        boxedActionButtonToneClass[tone],
        className,
      )}
      {...props}
    >
      {children}
    </Button>
  );
}

function categoryLabel(pluginType: string, t: Translate): string {
  switch (pluginType) {
    case "indexer": return t("settings.pluginCategoryIndexer");
    case "usenet_indexer": return t("settings.pluginCategoryUsenetIndexer");
    case "torrent_indexer": return t("settings.pluginCategoryTorrentIndexer");
    case "download_client": return t("settings.pluginCategoryDownloadClient");
    case "notification": return t("settings.pluginCategoryNotification");
    case "subtitle_provider": return t("settings.pluginCategorySubtitleProvider");
    default: return pluginType;
  }
}

function applyFilters(
  plugins: RegistryPluginRecord[],
  filters: FilterState,
): RegistryPluginRecord[] {
  return plugins
    .filter((p) => filters.category === "all" || p.pluginType === filters.category)
    .filter((p) => !filters.officialOnly || p.official)
    .sort((a, b) => a.name.localeCompare(b.name));
}

function PluginFilters({
  filters,
  categories,
  onChange,
}: {
  filters: FilterState;
  categories: string[];
  onChange: (filters: FilterState) => void;
}) {
  const t = useTranslate();
  return (
    <div className="flex items-center gap-3">
      <Select
        value={filters.category}
        onValueChange={(v) => onChange({ ...filters, category: v })}
      >
        <SelectTrigger className="h-8 w-44 text-sm">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="all">{t("settings.pluginAllCategories")}</SelectItem>
          {categories.map((cat) => (
            <SelectItem key={cat} value={cat}>
              {categoryLabel(cat, t)}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      <label className="flex cursor-pointer select-none items-center gap-1.5 text-sm text-muted-foreground">
        <Checkbox
          checked={filters.officialOnly}
          onCheckedChange={(checked) => onChange({ ...filters, officialOnly: !!checked })}
        />
        {t("settings.pluginOfficialOnly")}
      </label>
    </div>
  );
}

function PluginTable({
  plugins,
  mutatingPluginIds,
  pluginProgress,
  pluginErrors,
  showActions,
  onTogglePlugin,
  onInstallPlugin,
  onUninstallPlugin,
  onUpgradePlugin,
  installBlocked,
  upgradeBlocked,
  emptyMessage,
}: {
  plugins: RegistryPluginRecord[];
  mutatingPluginIds: string[];
  pluginProgress: Partial<Record<string, PluginInstallProgressRecord>>;
  pluginErrors: Partial<Record<string, string>>;
  showActions: "installed" | "available";
  onTogglePlugin: (plugin: RegistryPluginRecord) => void;
  onInstallPlugin: (plugin: RegistryPluginRecord) => void;
  onUninstallPlugin: (plugin: RegistryPluginRecord) => void;
  onUpgradePlugin: (plugin: RegistryPluginRecord) => void;
  installBlocked: boolean;
  upgradeBlocked: boolean;
  emptyMessage: string;
}) {
  const t = useTranslate();
  const nameColumnClass = "w-[38%]";
  const typeColumnClass = "w-[15%]";
  const versionColumnClass = "w-[13%]";
  const statusColumnClass = "w-[16%]";
  const actionsColumnClass =
    showActions === "installed" ? "w-32 text-right" : "w-48 text-right";
  if (plugins.length === 0) {
    return <p className="py-4 text-sm text-muted-foreground">{emptyMessage}</p>;
  }

  return (
    <div className="overflow-hidden rounded-xl border border-border/70 bg-card/20">
      <Table
        id={showActions === "installed" ? "settings-plugins-installed-table" : "settings-plugins-available-table"}
        className="table-fixed"
      >
        <TableHeader>
          <TableRow>
            <TableHead className={nameColumnClass}>{t("label.name")}</TableHead>
            <TableHead className={typeColumnClass}>{t("label.type")}</TableHead>
            <TableHead className={versionColumnClass}>{t("label.version")}</TableHead>
            <TableHead className={statusColumnClass}>{t("label.status")}</TableHead>
            {showActions === "installed" && (
              <TableHead className="w-20 text-center">{t("label.enabled")}</TableHead>
            )}
            <TableHead className={actionsColumnClass}>{t("label.actions")}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {plugins.map((plugin) => {
            const progress = pluginProgress[plugin.id];
            const runningProgress = isRunningPluginProgress(progress) ? progress : undefined;
            const isBusy = mutatingPluginIds.includes(plugin.id) || plugin.installInProgress;
            const isUpgrading =
              (runningProgress?.operationKind === "upgrade")
              || (plugin.installInProgress && showActions === "installed");
            const sourceLink = plugin.sourceRepo ?? plugin.sourceUrl;
            const normalizedSourceLink = normalizePluginLink(sourceLink);
            const normalizedDocsLink = normalizePluginLink(plugin.docsUrl);
            const showDocsLink =
              normalizedDocsLink !== null && normalizedDocsLink !== normalizedSourceLink;
            const actionError = pluginErrors[plugin.id];
            const displayVersion =
              showActions === "installed" && plugin.installedVersion
                ? plugin.installedVersion
                : plugin.version;
            const blockedLabel = blockedReasonLabel(plugin, t);
            const bytesLabel = formatPluginBytes(plugin.bytes);
            return (
              <TableRow
                key={plugin.id}
                id={selectorId("settings-plugin-row", plugin.name)}
              >
                <TableCell className={nameColumnClass}>
                  <div>
                    <div className="font-medium">{plugin.name}</div>
                    <div className="max-w-[300px] whitespace-normal break-words text-xs text-muted-foreground">
                      {plugin.description}
                    </div>
                    {(sourceLink || showDocsLink) && (
                      <div className="mt-2 flex flex-wrap items-center gap-2 text-xs">
                        {sourceLink && (
                          <a
                            href={sourceLink}
                            target="_blank"
                            rel="noreferrer"
                            className="inline-flex items-center gap-1 text-primary hover:underline"
                          >
                            {t("settings.pluginSource")}
                            <ExternalLink className="h-3 w-3" />
                          </a>
                        )}
                        {showDocsLink && plugin.docsUrl && (
                          <a
                            href={plugin.docsUrl}
                            target="_blank"
                            rel="noreferrer"
                            className="inline-flex items-center gap-1 text-primary hover:underline"
                          >
                            {t("settings.pluginDocs")}
                            <ExternalLink className="h-3 w-3" />
                          </a>
                        )}
                      </div>
                    )}
                  </div>
                </TableCell>
                <TableCell className={cn(typeColumnClass, "text-sm")}>{categoryLabel(plugin.pluginType, t)}</TableCell>
                <TableCell className={cn(versionColumnClass, "text-sm")}>
                  {t("settings.pluginVersion", { version: displayVersion })}
                  {bytesLabel && (
                    <div className="text-xs text-muted-foreground">
                      {t("settings.pluginBytes", { bytes: bytesLabel })}
                    </div>
                  )}
                  {plugin.updateAvailable && (
                    <div className="text-xs text-yellow-400">
                      {t("settings.pluginUpdateAvailable", { version: plugin.version })}
                    </div>
                  )}
                  {blockedLabel && (
                    <div className="text-xs text-destructive">
                      {blockedLabel}
                    </div>
                  )}
                  {actionError && (
                    <div className="text-xs text-destructive">
                    {actionError}
                  </div>
                )}
                </TableCell>
                <TableCell className={statusColumnClass}>
                  <div className="flex items-center gap-2">
                    {plugin.builtin && (
                      <span className="rounded bg-blue-900/40 px-1.5 py-0.5 text-xs text-blue-300">
                        {t("settings.pluginBuiltin")}
                      </span>
                    )}
                    {plugin.official && (
                      <span className="rounded bg-purple-900/40 px-1.5 py-0.5 text-xs text-purple-300">
                        {t("settings.pluginOfficial")}
                      </span>
                    )}
                    {plugin.supportTier === "verified_community" && (
                      <span className="rounded bg-cyan-900/40 px-1.5 py-0.5 text-xs text-cyan-300">
                        {t("settings.pluginVerifiedCommunity")}
                      </span>
                    )}
                    {plugin.supportTier === "unverified" && (
                      <span className="rounded bg-amber-900/40 px-1.5 py-0.5 text-xs text-amber-300">
                        {t("settings.pluginUnverified")}
                      </span>
                    )}
                    {isDownloadedBuiltinOverride(plugin) && (
                      <span className="rounded bg-amber-900/40 px-1.5 py-0.5 text-xs text-amber-300">
                        {t("settings.pluginOverride")}
                      </span>
                    )}
                  </div>
                </TableCell>
                {showActions === "installed" && (
                  <TableCell className="w-20 text-center">
                    <RenderBooleanIcon
                      value={plugin.isEnabled}
                      label={`${t("label.enabled")}: ${plugin.name}`}
                    />
                  </TableCell>
                )}
                <TableCell className={actionsColumnClass}>
                  {showActions === "installed" ? (
                    <div className="ml-auto flex min-w-0 flex-col items-end gap-2 w-28">
                      <div className="flex w-full items-center justify-end gap-1">
                        <PluginActionButton
                          id={selectorId("settings-plugin-toggle", plugin.name)}
                          tone={plugin.isEnabled ? "disabled" : "enabled"}
                          disabled={isBusy}
                          onClick={() => onTogglePlugin(plugin)}
                          label={plugin.isEnabled ? t("label.disable") : t("label.enable")}
                        >
                          {plugin.isEnabled ? (
                            <PowerOff className="h-4 w-4" />
                          ) : (
                            <Power className="h-4 w-4" />
                          )}
                        </PluginActionButton>
                        {plugin.updateAvailable && (
                          <PluginActionButton
                            id={selectorId("settings-plugin-upgrade", plugin.name)}
                            tone="upgrade"
                            disabled={isBusy || upgradeBlocked}
                            onClick={() => onUpgradePlugin(plugin)}
                            label={t("settings.pluginUpgrade", { version: plugin.version })}
                          >
                            {isUpgrading ? (
                              <Loader2 className="h-4 w-4 animate-spin" />
                            ) : (
                              <ArrowUpCircle className="h-4 w-4" />
                            )}
                          </PluginActionButton>
                        )}
                        {canUninstallPlugin(plugin) && (
                          <PluginActionButton
                            id={selectorId("settings-plugin-uninstall", plugin.name)}
                            tone="delete"
                            disabled={isBusy}
                            onClick={() => onUninstallPlugin(plugin)}
                            label={uninstallLabel(plugin, t)}
                          >
                            <Trash2 className="h-4 w-4" />
                          </PluginActionButton>
                        )}
                      </div>
                      {runningProgress && (
                        <PluginInstallProgressBar
                          progress={runningProgress}
                          id={selectorId("settings-plugin-progress", plugin.name)}
                          className="w-full space-y-1 overflow-hidden"
                        />
                      )}
                    </div>
                  ) : (
                    <div className="ml-auto grid w-44 grid-cols-[minmax(0,1fr)_auto] items-center gap-3">
                      <div className="min-w-0">
                        {runningProgress ? (
                          <PluginInstallProgressBar progress={runningProgress} />
                        ) : null}
                      </div>
                      <div className="flex justify-end">
                        <PluginActionButton
                          id={selectorId("settings-plugin-install", plugin.name)}
                          tone="install"
                          disabled={isBusy || installBlocked || installIsBlocked(plugin)}
                          onClick={() => onInstallPlugin(plugin)}
                          label={t("settings.pluginInstall")}
                        >
                          {isBusy ? (
                            <Loader2 className="h-4 w-4 animate-spin" />
                          ) : (
                            <Download className="h-4 w-4" />
                          )}
                        </PluginActionButton>
                      </div>
                    </div>
                  )}
                </TableCell>
              </TableRow>
            );
          })}
        </TableBody>
      </Table>
    </div>
  );
}

export function SettingsPluginsSection({
  plugins,
  catalogStatus,
  initialLoading,
  mutatingPluginIds,
  pluginProgress,
  pluginErrors,
  refreshing,
  upgradingAll,
  manualRepoUrl,
  manualFileName,
  manualPreview,
  manualBusy,
  showManualInstall,
  remoteActionsBlocked,
  onManualRepoUrlChange,
  onToggleManualInstall,
  onManualPluginFileChange,
  onInspectManualPluginRepo,
  onRequestInstallUploadedPlugin,
  onInstallManualPlugin,
  onRefreshRegistry,
  onUpgradeAllPlugins,
  onTogglePlugin,
  onInstallPlugin,
  onUninstallPlugin,
  onUpgradePlugin,
}: SettingsPluginsSectionProps) {
  const t = useTranslate();
  const [installedFilters, setInstalledFilters] = useState<FilterState>({
    category: "all",
    officialOnly: false,
  });
  const manualPluginFileInputRef = useRef<HTMLInputElement | null>(null);
  const [availableFilters, setAvailableFilters] = useState<FilterState>({
    category: "all",
    officialOnly: false,
  });

  const installed = useMemo(() => plugins.filter((p) => p.isInstalled), [plugins]);
  const available = useMemo(() => plugins.filter((p) => !p.isInstalled), [plugins]);
  const allCategories = useMemo(
    () => [...new Set(plugins.map((p) => p.pluginType))].sort(),
    [plugins],
  );

  const filteredInstalled = useMemo(
    () => applyFilters(installed, installedFilters),
    [installed, installedFilters],
  );
  const filteredAvailable = useMemo(
    () => applyFilters(available, availableFilters),
    [available, availableFilters],
  );

  const upgradeCount = installed.filter((p) => p.updateAvailable).length;

  return (
    <div className="space-y-8">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <p className="text-sm text-muted-foreground">{t("settings.pluginsSection")}</p>
          {upgradeCount > 0 && (
            <span className="inline-flex h-5 min-w-5 items-center justify-center rounded-full bg-red-600 px-1.5 text-[11px] font-medium text-white">
              {upgradeCount}
            </span>
          )}
        </div>
        <div className="flex items-center gap-2">
          <Button
            id="settings-plugins-upgrade-all"
            variant="outline"
            size="sm"
            disabled={upgradingAll || remoteActionsBlocked.upgrade || upgradeCount === 0}
            onClick={onUpgradeAllPlugins}
          >
            <ArrowUpCircle className={`mr-2 h-4 w-4 ${upgradingAll ? "animate-spin" : ""}`} />
            {upgradingAll ? t("settings.pluginsUpdatingAll") : t("settings.pluginsUpdateAll")}
          </Button>
          <Button
            id="settings-plugins-manual-toggle"
            variant="outline"
            size="sm"
            onClick={onToggleManualInstall}
          >
            {t("settings.pluginInstallManually")}
          </Button>
          <Button
            id="settings-plugins-refresh"
            variant="outline"
            size="sm"
            disabled={refreshing || remoteActionsBlocked.refresh}
            onClick={onRefreshRegistry}
          >
            <RefreshCw className={`mr-2 h-4 w-4 ${refreshing ? "animate-spin" : ""}`} />
            {refreshing ? t("label.refreshing") : t("settings.pluginsRefresh")}
          </Button>
        </div>
      </div>

      {catalogStatus?.outageMessage && (
        <div className="rounded-lg border border-amber-500/30 bg-amber-500/10 p-3 text-sm text-amber-100">
          {catalogStatus.outageMessage}
        </div>
      )}

      {catalogStatus && catalogStatus.restoreWarnings.length > 0 && (
        <div className="rounded-lg border border-amber-500/30 bg-amber-500/10 p-3 text-sm text-amber-100">
          <div className="font-medium">Restore warnings</div>
          <ul className="mt-2 list-disc space-y-1 pl-5">
            {catalogStatus.restoreWarnings.map((warning) => (
              <li key={warning}>{warning}</li>
            ))}
          </ul>
        </div>
      )}

      {initialLoading ? (
        <div className="flex min-h-48 items-center justify-center rounded-xl border border-border/70 bg-card/40">
          <div className="flex items-center gap-3 text-sm text-muted-foreground">
            <Loader2 className="h-5 w-5 animate-spin" />
            <span>{t("label.loading")}</span>
          </div>
        </div>
      ) : null}

      {!initialLoading && showManualInstall && (
        <div className="rounded-xl border border-border bg-card/60 p-4">
          <div className="space-y-5">
            <div className="space-y-3">
              <div className="space-y-1">
                <h3 className="text-sm font-medium">{t("settings.pluginManualUploadTitle")}</h3>
                <p className="text-sm text-muted-foreground">
                  {t("settings.pluginManualUploadHelp")}
                </p>
              </div>
              <input
                id="settings-plugins-manual-file-input"
                ref={manualPluginFileInputRef}
                type="file"
                accept=".wasm,.zst"
                className="hidden"
                onChange={onManualPluginFileChange}
              />
              <div className="flex flex-col gap-3 md:flex-row md:items-center">
                <Button
                  id="settings-plugins-manual-file-select"
                  type="button"
                  variant="outline"
                  onClick={() => manualPluginFileInputRef.current?.click()}
                  disabled={manualBusy}
                >
                  <Upload className="mr-2 h-4 w-4" />
                  {t("settings.pluginManualUploadSelect")}
                </Button>
                <Button
                  id="settings-plugins-manual-file-install"
                  type="button"
                  disabled={manualBusy || !manualFileName}
                  onClick={onRequestInstallUploadedPlugin}
                >
                  {manualBusy ? t("label.loading") : t("settings.pluginManualUploadInstall")}
                </Button>
              </div>
              <div className="space-y-1 text-sm text-muted-foreground">
                <p id="settings-plugins-manual-file-name">
                  {manualFileName ?? t("settings.pluginManualUploadNoFile")}
                </p>
                <p>{t("settings.pluginManualUploadFormats")}</p>
              </div>
              {pluginErrors.__manualUpload && (
                <p className="text-sm text-destructive">{pluginErrors.__manualUpload}</p>
              )}
            </div>

            <div className="border-t border-border pt-5">
              <div className="space-y-3">
                <div className="space-y-1">
                  <h3 className="text-sm font-medium">{t("settings.pluginManualRepoTitle")}</h3>
                  <p className="text-sm text-muted-foreground">
                    {t("settings.pluginManualRepoHelp")}
                  </p>
                </div>
                <div className="flex flex-col gap-3 md:flex-row md:items-end">
                  <label className="flex-1 space-y-1 text-sm">
                    <span className="font-medium">{t("settings.pluginManualRepoUrl")}</span>
                    <input
                      id="settings-plugins-manual-repo-url"
                      value={manualRepoUrl}
                      onChange={(event) => onManualRepoUrlChange(event.target.value)}
                      placeholder="https://github.com/example/scryer-plugin-example"
                      className="h-10 w-full rounded-md border border-input bg-background px-3 text-sm"
                    />
                  </label>
                  <Button
                    id="settings-plugins-manual-repo-inspect"
                    type="button"
                    disabled={manualBusy || remoteActionsBlocked.inspectManual || !manualRepoUrl.trim()}
                    onClick={onInspectManualPluginRepo}
                  >
                    {manualBusy ? t("label.loading") : t("settings.pluginInspectManual")}
                  </Button>
                </div>
              </div>
            </div>
          </div>
          {pluginErrors.__manual && (
            <p className="mt-2 text-sm text-destructive">{pluginErrors.__manual}</p>
          )}
          {manualPreview && (
            <div className="mt-3 flex flex-col gap-3 rounded-lg border border-border p-3 md:flex-row md:items-center md:justify-between">
              <div>
                <div className="font-medium">{manualPreview.plugin.name}</div>
                <div className="text-sm text-muted-foreground">
                  {manualPreview.plugin.description}
                </div>
                <div className="mt-1 text-xs text-amber-300">
                  {t("settings.pluginUnverified")}
                </div>
              </div>
              <Button
                id="settings-plugins-manual-repo-install"
                type="button"
                disabled={manualBusy || remoteActionsBlocked.installManual}
                onClick={onInstallManualPlugin}
              >
                {t("settings.pluginInstall")}
              </Button>
            </div>
          )}
        </div>
      )}

      {!initialLoading && plugins.length === 0 ? (
        <p className="py-4 text-sm text-muted-foreground">{t("settings.pluginsNoPlugins")}</p>
      ) : !initialLoading ? (
        <>
          <div className="space-y-3">
            <div className="flex items-center justify-between">
              <h3 className="text-sm font-medium">{t("settings.pluginsInstalled")}</h3>
              <PluginFilters
                filters={installedFilters}
                categories={allCategories}
                onChange={setInstalledFilters}
              />
            </div>
              <PluginTable
                plugins={filteredInstalled}
                mutatingPluginIds={mutatingPluginIds}
                pluginProgress={pluginProgress}
                pluginErrors={pluginErrors}
                showActions="installed"
                onTogglePlugin={onTogglePlugin}
                onInstallPlugin={onInstallPlugin}
                onUninstallPlugin={onUninstallPlugin}
                onUpgradePlugin={onUpgradePlugin}
                installBlocked={remoteActionsBlocked.install}
                upgradeBlocked={remoteActionsBlocked.upgrade}
                emptyMessage={t("settings.pluginsNoInstalled")}
              />
          </div>

          <div className="space-y-3">
            <div className="flex items-center justify-between">
              <h3 className="text-sm font-medium">{t("settings.pluginsAvailable")}</h3>
              <PluginFilters
                filters={availableFilters}
                categories={allCategories}
                onChange={setAvailableFilters}
              />
            </div>
              <PluginTable
                plugins={filteredAvailable}
                mutatingPluginIds={mutatingPluginIds}
                pluginProgress={pluginProgress}
                pluginErrors={pluginErrors}
                showActions="available"
                onTogglePlugin={onTogglePlugin}
                onInstallPlugin={onInstallPlugin}
                onUninstallPlugin={onUninstallPlugin}
                onUpgradePlugin={onUpgradePlugin}
                installBlocked={remoteActionsBlocked.install}
                upgradeBlocked={remoteActionsBlocked.upgrade}
                emptyMessage={t("settings.pluginsNoAvailable")}
              />
          </div>
        </>
      ) : null}
    </div>
  );
}
