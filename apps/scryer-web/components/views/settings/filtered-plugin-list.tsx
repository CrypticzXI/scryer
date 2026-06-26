import * as React from "react";
import { useClient } from "urql";
import {
  ArrowUpCircle,
  Loader2,
  Plus,
  Power,
  PowerOff,
  RefreshCw,
  Trash2,
} from "lucide-react";
import type { ProviderCatalogFamily } from "@/lib/hooks/use-provider-catalog-subscription";
import { usePluginManagement } from "@/lib/hooks/use-plugin-management";
import {
  isRunningPluginProgress,
  PluginInstallProgressBar,
} from "@/components/views/settings/settings-plugins-section";
import { useTranslate } from "@/lib/context/translate-context";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { cn } from "@/lib/utils";

/** Plugin `pluginType` values that belong to each provider-catalog family. */
const FAMILY_PLUGIN_TYPES: Record<ProviderCatalogFamily, string[]> = {
  indexer: ["indexer", "usenet_indexer", "torrent_indexer"],
  download_client: ["download_client"],
  notification: ["notification"],
  subtitle: ["subtitle_provider"],
};

export type FilteredPluginListProps = {
  /** Plugin family to show + manage (e.g. "indexer"). */
  family: ProviderCatalogFamily;
  /** Refreshes provider options after a plugin change so new providers appear. */
  refreshProviderOptions: () => Promise<void>;
  /** Panel heading; defaults to "Plugins". */
  title?: string;
  className?: string;
};

/**
 * A self-contained, family-filtered plugin manager for embedding on per-type
 * settings surfaces (indexers, download clients, notifications, subtitles). It
 * lists the family's plugins and lets the user enable/disable, install,
 * uninstall, and upgrade them in place — the same management the full Plugins
 * page offers, scoped to one family.
 */
export function FilteredPluginList({
  family,
  refreshProviderOptions,
  title,
  className,
}: FilteredPluginListProps) {
  const client = useClient();
  const t = useTranslate();
  const {
    plugins,
    pluginsLoading,
    pluginsRefreshing,
    mutatingPluginIds,
    pluginProgress,
    pluginErrors,
    pluginsError,
    refreshPluginsRegistry,
    installPlugin,
    uninstallPlugin,
    togglePlugin,
    upgradePlugin,
  } = usePluginManagement({ client, t, refreshProviderOptions });

  const allowedTypes = FAMILY_PLUGIN_TYPES[family];
  const familyPlugins = React.useMemo(
    () =>
      plugins
        .filter((plugin) => allowedTypes.includes(plugin.pluginType))
        .sort(
          (a, b) =>
            Number(b.isInstalled) - Number(a.isInstalled) ||
            a.name.localeCompare(b.name),
        ),
    [plugins, allowedTypes],
  );

  return (
    <Card className={cn("flex min-h-0 flex-col", className)}>
      <CardHeader className="flex flex-row items-center justify-between gap-2 space-y-0">
        <CardTitle className="text-base">
          {title ?? t("settings.plugins")}
        </CardTitle>
        <Button
          type="button"
          variant="secondary"
          size="icon-sm"
          onClick={() => void refreshPluginsRegistry()}
          disabled={pluginsRefreshing}
          title={t("settings.pluginsRefresh")}
          aria-label={t("settings.pluginsRefresh")}
        >
          <RefreshCw
            className={cn("h-4 w-4", pluginsRefreshing && "animate-spin")}
          />
        </Button>
      </CardHeader>
      <CardContent className="space-y-2">
        {pluginsError ? (
          <p className="text-xs text-red-400">{pluginsError}</p>
        ) : null}
        {pluginsLoading ? (
          <div className="flex items-center gap-2 py-6 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            {t("label.loading")}
          </div>
        ) : familyPlugins.length === 0 ? (
          <p className="py-6 text-sm text-muted-foreground">
            {t("settings.pluginsNoAvailable")}
          </p>
        ) : (
          familyPlugins.map((plugin) => {
            const mutating = mutatingPluginIds.includes(plugin.id);
            const progress = pluginProgress[plugin.id];
            const running = Boolean(
              progress && isRunningPluginProgress(progress),
            );
            const error = pluginErrors[plugin.id];
            const canUninstall = plugin.isInstalled && !plugin.builtin;
            const hasStatusBadges =
              plugin.builtin ||
              plugin.official ||
              plugin.supportTier === "verified_community" ||
              plugin.supportTier === "unverified" ||
              plugin.status === "beta";
            return (
              <div
                key={plugin.id}
                className="rounded-[10px] border border-[var(--scry-border2)] bg-[var(--scry-inset)] p-3"
              >
                <div className="flex items-start justify-between gap-2">
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      {plugin.isInstalled && plugin.isEnabled ? (
                        <span
                          className="h-1.5 w-1.5 shrink-0 rounded-full bg-emerald-400"
                          aria-hidden="true"
                        />
                      ) : null}
                      <span className="truncate text-[13px] font-semibold text-[var(--scry-ink2)]">
                        {plugin.name}
                      </span>
                    </div>
                    {plugin.description ? (
                      <p className="mt-0.5 line-clamp-2 text-[11.5px] leading-snug text-[var(--scry-muted)]">
                        {plugin.description}
                      </p>
                    ) : null}
                    {hasStatusBadges ? (
                      <div className="mt-1.5 flex flex-wrap items-center gap-1">
                        {plugin.builtin ? (
                          <Badge tone="info">{t("settings.pluginBuiltin")}</Badge>
                        ) : null}
                        {plugin.official ? (
                          <Badge tone="info">{t("settings.pluginOfficial")}</Badge>
                        ) : null}
                        {plugin.supportTier === "verified_community" ? (
                          <Badge tone="positive">
                            {t("settings.pluginVerifiedCommunity")}
                          </Badge>
                        ) : null}
                        {plugin.supportTier === "unverified" ? (
                          <Badge tone="warning">
                            {t("settings.pluginUnverified")}
                          </Badge>
                        ) : null}
                        {plugin.status === "beta" ? (
                          <Badge tone="warning">{t("settings.pluginBeta")}</Badge>
                        ) : null}
                        {plugin.builtin && plugin.sourceKind === "downloaded" ? (
                          <Badge tone="warning">
                            {t("settings.pluginOverride")}
                          </Badge>
                        ) : null}
                      </div>
                    ) : null}
                  </div>
                  {running ? null : (
                    <div className="flex shrink-0 items-center gap-1">
                      {plugin.isInstalled ? (
                        <>
                          <Button
                            type="button"
                            variant="secondary"
                            size="icon-sm"
                            onClick={() => void togglePlugin(plugin)}
                            disabled={mutating}
                            title={
                              plugin.isEnabled
                                ? t("label.disable")
                                : t("label.enable")
                            }
                            aria-label={
                              plugin.isEnabled
                                ? t("label.disable")
                                : t("label.enable")
                            }
                          >
                            {plugin.isEnabled ? (
                              <PowerOff className="h-3.5 w-3.5" />
                            ) : (
                              <Power className="h-3.5 w-3.5" />
                            )}
                          </Button>
                          {plugin.updateAvailable ? (
                            <Button
                              type="button"
                              variant="secondary"
                              size="icon-sm"
                              onClick={() => void upgradePlugin(plugin)}
                              disabled={mutating}
                              title={t("settings.pluginUpgrade", {
                                version: plugin.latestVersion ?? plugin.version,
                              })}
                              aria-label={t("settings.pluginUpgrade", {
                                version: plugin.latestVersion ?? plugin.version,
                              })}
                            >
                              <ArrowUpCircle className="h-3.5 w-3.5 text-[var(--scry-accent-text)]" />
                            </Button>
                          ) : null}
                          {canUninstall ? (
                            <Button
                              type="button"
                              variant="secondary"
                              size="icon-sm"
                              onClick={() => void uninstallPlugin(plugin)}
                              disabled={mutating}
                              title={t("settings.pluginUninstall")}
                              aria-label={t("settings.pluginUninstall")}
                            >
                              <Trash2 className="h-3.5 w-3.5 text-red-400" />
                            </Button>
                          ) : null}
                        </>
                      ) : (
                        <Button
                          type="button"
                          variant="primary"
                          size="sm"
                          onClick={() => void installPlugin(plugin)}
                          disabled={mutating}
                        >
                          {mutating ? (
                            <Loader2 className="h-3.5 w-3.5 animate-spin" />
                          ) : (
                            <Plus className="h-3.5 w-3.5" />
                          )}
                          {t("settings.pluginInstall")}
                        </Button>
                      )}
                    </div>
                  )}
                </div>
                {running && progress ? (
                  <div className="mt-2">
                    <PluginInstallProgressBar progress={progress} />
                  </div>
                ) : null}
                {error ? (
                  <p className="mt-1.5 text-[11px] text-red-400">{error}</p>
                ) : null}
              </div>
            );
          })
        )}
      </CardContent>
    </Card>
  );
}
