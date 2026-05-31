import { Fragment } from "react";
import { Download, Loader2, PlugZap, RefreshCw, Trash2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { formatPluginBytes } from "@/components/views/settings/settings-plugins-section";
import type {
  PluginInstallProgressRecord,
  RegistryPluginRecord,
} from "@/components/views/settings/settings-plugins-section";
import { selectorId } from "@/lib/utils/dom-ids";

interface SetupPluginsViewProps {
  t: (
    key: string,
    values?: Record<string, string | number | boolean | null | undefined>,
  ) => string;
  plugins: RegistryPluginRecord[];
  loading: boolean;
  refreshing: boolean;
  mutatingPluginIds: string[];
  pluginProgress: Partial<Record<string, PluginInstallProgressRecord>>;
  pluginErrors: Partial<Record<string, string>>;
  error: string | null;
  onRefreshRegistry: () => void;
  onInstallPlugin: (plugin: RegistryPluginRecord) => void;
  onUninstallPlugin: (plugin: RegistryPluginRecord) => void;
  onNext: () => void;
  onBack: () => void;
}

function categoryLabel(
  pluginType: string,
  t: (
    key: string,
    values?: Record<string, string | number | boolean | null | undefined>,
  ) => string,
) {
  if (pluginType === "indexer" || pluginType.endsWith("_indexer")) {
    return t("settings.pluginCategoryIndexer");
  }
  if (pluginType === "download_client") {
    return t("settings.pluginCategoryDownloadClient");
  }
  if (pluginType === "notification") {
    return t("settings.pluginCategoryNotification");
  }
  return pluginType;
}

function categoryKey(pluginType: string) {
  if (pluginType === "indexer" || pluginType.endsWith("_indexer")) {
    return "indexer";
  }
  if (pluginType === "download_client") {
    return "download_client";
  }
  if (pluginType === "notification") {
    return "notification";
  }
  return pluginType;
}

function groupPluginsByType(
  plugins: RegistryPluginRecord[],
  t: (
    key: string,
    values?: Record<string, string | number | boolean | null | undefined>,
  ) => string,
) {
  const groups = new Map<
    string,
    { label: string; plugins: RegistryPluginRecord[] }
  >();

  for (const plugin of plugins) {
    const key = categoryKey(plugin.pluginType);
    const existing = groups.get(key);
    if (existing) {
      existing.plugins.push(plugin);
      continue;
    }
    groups.set(key, {
      label: categoryLabel(key, t),
      plugins: [plugin],
    });
  }

  return [...groups.entries()]
    .map(([key, value]) => ({
      key,
      label: value.label,
      plugins: value.plugins.sort((left, right) =>
        left.name.localeCompare(right.name),
      ),
    }))
    .sort((left, right) => left.label.localeCompare(right.label));
}

function canUninstallPlugin(plugin: RegistryPluginRecord) {
  return !plugin.builtin || plugin.sourceKind === "downloaded";
}

function uninstallLabel(plugin: RegistryPluginRecord, t: SetupPluginsViewProps["t"]) {
  return plugin.builtin && plugin.sourceKind === "downloaded"
    ? t("settings.pluginRevertToBundled")
    : t("settings.pluginUninstall");
}

function installIsBlocked(plugin: RegistryPluginRecord): boolean {
  return plugin.blockedReason === "no_compatible_release";
}

function blockedReasonLabel(
  plugin: RegistryPluginRecord,
  t: SetupPluginsViewProps["t"],
): string | null {
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

function isRunningPluginProgress(
  progress?: PluginInstallProgressRecord,
): progress is PluginInstallProgressRecord {
  return progress !== undefined
    && progress.state !== "succeeded"
    && progress.state !== "failed";
}

function pluginProgressLabel(
  progress: PluginInstallProgressRecord,
  t: SetupPluginsViewProps["t"],
): string {
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

export function SetupPluginsView({
  t,
  plugins,
  loading,
  refreshing,
  mutatingPluginIds,
  pluginProgress,
  pluginErrors,
  error,
  onRefreshRegistry,
  onInstallPlugin,
  onUninstallPlugin,
  onNext,
  onBack,
}: SetupPluginsViewProps) {
  const groupedPlugins = groupPluginsByType(
    plugins.filter((plugin) => plugin.official),
    t,
  );

  return (
    <div id="setup-plugins-view" className="flex flex-col gap-6">
      <div className="text-center">
        <h2 className="text-xl font-semibold">{t("setup.pluginsTitle")}</h2>
        <p className="mt-1 text-sm text-muted-foreground">
          {t("setup.pluginsDescription")}
        </p>
      </div>

      <div className="mx-auto w-full max-w-5xl rounded-xl border border-dashed border-border bg-muted/30 px-4 py-3 text-sm">
        <span className="font-medium">{t("setup.pluginsBuiltInTitle")}:</span>{" "}
        <span className="text-muted-foreground">
          {t("setup.pluginsBuiltInDescription")}
        </span>
      </div>

      <div className="mx-auto flex w-full max-w-5xl items-center justify-between gap-4">
        <div>
          <p className="text-sm font-medium">
            {t("setup.pluginsAvailableHeading")}
          </p>
          <p className="text-sm text-muted-foreground">
            {t("setup.pluginsAvailableHint")}
          </p>
        </div>
        <Button
          id="setup-plugins-refresh"
          variant="outline"
          size="sm"
          disabled={refreshing || loading}
          onClick={onRefreshRegistry}
        >
          {refreshing ? (
            <Loader2 className="mr-2 h-4 w-4 animate-spin" />
          ) : (
            <RefreshCw className="mr-2 h-4 w-4" />
          )}
          {refreshing ? t("label.refreshing") : t("label.refresh")}
        </Button>
      </div>

      {error && (
        <p className="mx-auto w-full max-w-5xl text-sm text-destructive">
          {error}
        </p>
      )}

      {loading ? (
        <div className="mx-auto flex w-full max-w-5xl items-center justify-center gap-2 rounded-xl border border-dashed border-border py-10 text-sm text-muted-foreground">
          <Loader2 className="h-4 w-4 animate-spin" />
          {t("label.loading")}
        </div>
      ) : (
        <div className="mx-auto w-full max-w-5xl">
          {groupedPlugins.length === 0 ? (
            <div className="rounded-xl border border-dashed border-border py-10 text-center text-sm text-muted-foreground">
              {t("setup.pluginsNoneFound")}
            </div>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{t("label.name")}</TableHead>
                  <TableHead className="w-[120px]">
                    {t("queue.size")}
                  </TableHead>
                  <TableHead className="w-[140px] text-right">
                    {t("label.actions")}
                  </TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {groupedPlugins.map((group) => (
                  <Fragment key={group.key}>
                    <TableRow className="bg-muted/35 hover:bg-muted/35">
                      <TableCell colSpan={3}>
                        <span className="text-xs font-semibold uppercase tracking-[0.16em] text-muted-foreground">
                          {group.label}
                        </span>
                      </TableCell>
                    </TableRow>
                    {group.plugins.map((plugin) => {
                      const progress = pluginProgress[plugin.id];
                      const runningProgress = isRunningPluginProgress(progress) ? progress : undefined;
                      const isBusy = mutatingPluginIds.includes(plugin.id) || plugin.installInProgress;
                      const actionError = pluginErrors[plugin.id];
                      const blockedLabel = blockedReasonLabel(plugin, t);
                      const bytesLabel = formatPluginBytes(plugin.bytes);
                      return (
                        <TableRow
                          key={plugin.id}
                          id={selectorId("setup-plugin-row", plugin.name)}
                        >
                          <TableCell className="min-w-[260px]">
                            <div className="space-y-1">
                              <span className="font-medium">{plugin.name}</span>
                              <p className="whitespace-normal break-words text-xs text-muted-foreground">
                                {plugin.description}
                              </p>
                              {(blockedLabel || actionError) && (
                                <div className="space-y-1">
                                  {blockedLabel && (
                                    <p className="text-xs text-destructive">
                                      {blockedLabel}
                                    </p>
                                  )}
                                  {actionError && (
                                    <p className="text-xs text-destructive">
                                      {actionError}
                                    </p>
                                  )}
                                </div>
                              )}
                            </div>
                          </TableCell>
                          <TableCell
                            className="w-[120px] text-sm text-muted-foreground"
                            title={plugin.bytes != null ? `${plugin.bytes} bytes` : undefined}
                          >
                            {bytesLabel ?? "—"}
                          </TableCell>
                          <TableCell className="w-[124px] text-right">
                            {plugin.isInstalled ? (
                              <div className="ml-auto flex w-28 min-w-0 flex-col items-end gap-2">
                                <div className="flex w-full items-center justify-end gap-2">
                                  <span className="text-sm text-muted-foreground">
                                    {t("settings.pluginInstalled")}
                                  </span>
                                  {canUninstallPlugin(plugin) && (
                                    <Button
                                      id={selectorId("setup-plugin-uninstall", plugin.name)}
                                      variant="ghost"
                                      size="icon"
                                      disabled={isBusy}
                                      onClick={() => onUninstallPlugin(plugin)}
                                      title={uninstallLabel(plugin, t)}
                                    >
                                      {isBusy ? (
                                        <Loader2 className="h-4 w-4 animate-spin text-destructive" />
                                      ) : (
                                        <Trash2 className="h-4 w-4 text-destructive" />
                                      )}
                                    </Button>
                                  )}
                                </div>
                                {runningProgress && (
                                  <div className="w-full space-y-1 overflow-hidden">
                                    <div className="truncate text-right text-xs leading-tight text-primary">
                                      {pluginProgressLabel(runningProgress, t)}
                                    </div>
                                    <Progress
                                      value={(runningProgress.stepIndex / Math.max(runningProgress.stepCount, 1)) * 100}
                                      className="h-1.5"
                                    />
                                  </div>
                                )}
                              </div>
                            ) : (
                              <div className="ml-auto flex w-24 min-w-0 flex-col items-end gap-2">
                                <Button
                                  id={selectorId("setup-plugin-install", plugin.name)}
                                  variant="outline"
                                  size="sm"
                                  disabled={isBusy || installIsBlocked(plugin)}
                                  onClick={() => onInstallPlugin(plugin)}
                                >
                                  {isBusy ? (
                                    <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                                  ) : (
                                    <Download className="mr-2 h-4 w-4" />
                                  )}
                                  {isBusy
                                    ? t("settings.pluginInstalling")
                                    : t("settings.pluginInstall")}
                                </Button>
                                {runningProgress && (
                                  <div className="w-full space-y-1 overflow-hidden">
                                    <div className="truncate text-right text-xs leading-tight text-primary">
                                      {pluginProgressLabel(runningProgress, t)}
                                    </div>
                                    <Progress
                                      value={(runningProgress.stepIndex / Math.max(runningProgress.stepCount, 1)) * 100}
                                      className="h-1.5"
                                    />
                                  </div>
                                )}
                              </div>
                            )}
                          </TableCell>
                        </TableRow>
                      );
                    })}
                  </Fragment>
                ))}
              </TableBody>
            </Table>
          )}
        </div>
      )}

      <div className="flex items-center justify-between pt-2">
        <Button id="setup-plugins-back" variant="ghost" onClick={onBack}>
          {t("setup.back")}
        </Button>
        <Button id="setup-plugins-next" onClick={onNext}>
          <PlugZap className="mr-2 h-4 w-4" />
          {t("setup.next")}
        </Button>
      </div>
    </div>
  );
}
