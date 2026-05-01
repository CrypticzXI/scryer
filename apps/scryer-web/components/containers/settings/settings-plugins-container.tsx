import { useCallback, useEffect, useState } from "react";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import {
  SettingsPluginsSection,
  type RegistryPluginRecord,
} from "@/components/views/settings/settings-plugins-section";
import { useClient } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { dispatchNavigationBadgesRefresh } from "@/lib/events/navigation-badges";
import { pluginsQuery } from "@/lib/graphql/queries";
import {
  refreshPluginRegistryMutation,
  installPluginMutation,
  uninstallPluginMutation,
  togglePluginMutation,
  upgradePluginMutation,
} from "@/lib/graphql/mutations";

function extractPluginMutationErrorMessage(error: unknown): string | null {
  if (error && typeof error === "object" && "graphQLErrors" in error) {
    const graphQLErrors = (error as { graphQLErrors?: Array<{ message?: string }> }).graphQLErrors;
    const message = graphQLErrors?.find((entry) => typeof entry.message === "string")?.message;
    if (message?.trim()) {
      return message.trim();
    }
  }

  if (error instanceof Error && error.message.trim()) {
    return error.message.trim();
  }

  return null;
}

function formatPluginInstallError(
  plugin: RegistryPluginRecord,
  error: unknown,
  t: ReturnType<typeof useTranslate>,
): string {
  const rawMessage = extractPluginMutationErrorMessage(error);
  const normalized = rawMessage
    ?.replace(/^\[GraphQL\]\s*/i, "")
    .replace(/^validation:\s*/i, "")
    .trim();

  if (normalized && /WASM SHA256 mismatch/i.test(normalized)) {
    return t("status.pluginInstallFailedChecksumMismatch", { name: plugin.name });
  }

  if (
    normalized
    && normalized.includes("has sdk_constraint")
    && normalized.includes("but registry selected")
  ) {
    return t("status.pluginInstallFailedSdkMetadataMismatch", { name: plugin.name });
  }

  if (normalized) {
    return t("status.pluginInstallFailedWithReason", {
      name: plugin.name,
      reason: normalized,
    });
  }

  return t("status.failedToUpdate");
}

export function SettingsPluginsContainer() {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [plugins, _setPlugins] = useState<RegistryPluginRecord[]>([]);
  const [pluginErrors, setPluginErrors] = useState<Record<string, string>>({});

  const setPlugins = useCallback((next: RegistryPluginRecord[]) => {
    _setPlugins(next);
  }, []);
  const [mutatingPluginIds, setMutatingPluginIds] = useState<string[]>([]);
  const [upgradingPluginIds, setUpgradingPluginIds] = useState<string[]>([]);
  const [refreshing, setRefreshing] = useState(false);
  const [pendingUninstall, setPendingUninstall] = useState<RegistryPluginRecord | null>(null);

  const beginPluginMutation = useCallback((pluginId: string) => {
    setMutatingPluginIds((current) => (
      current.includes(pluginId) ? current : [...current, pluginId]
    ));
  }, []);

  const endPluginMutation = useCallback((pluginId: string) => {
    setMutatingPluginIds((current) => current.filter((id) => id !== pluginId));
  }, []);

  const beginPluginUpgrade = useCallback((pluginId: string) => {
    setUpgradingPluginIds((current) => (
      current.includes(pluginId) ? current : [...current, pluginId]
    ));
  }, []);

  const endPluginUpgrade = useCallback((pluginId: string) => {
    setUpgradingPluginIds((current) => current.filter((id) => id !== pluginId));
  }, []);

  const refreshPlugins = useCallback(async () => {
    try {
      const { data, error } = await client.query(pluginsQuery, {}).toPromise();
      if (error) throw error;
      setPlugins(data.plugins || []);
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    }
  }, [client, setGlobalStatus, t, setPlugins]);

  useEffect(() => {
    void refreshPlugins();
  }, [refreshPlugins]);

  const refreshRegistry = async () => {
    setRefreshing(true);
    try {
      const { data, error } = await client
        .mutation(refreshPluginRegistryMutation, {})
        .toPromise();
      if (error) throw error;
      setPlugins(data.refreshPluginRegistry || []);
      dispatchNavigationBadgesRefresh();
      setGlobalStatus(t("status.registryRefreshed"));
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    } finally {
      setRefreshing(false);
    }
  };

  const togglePlugin = useCallback(
    async (plugin: RegistryPluginRecord) => {
      beginPluginMutation(plugin.id);
      try {
        const { error } = await client
          .mutation(togglePluginMutation, {
            input: { pluginId: plugin.id, enabled: !plugin.isEnabled },
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(
          t("status.pluginToggled", {
            name: plugin.name,
            state: plugin.isEnabled ? t("label.disabled") : t("label.enabled"),
          }),
        );
        await refreshPlugins();
        dispatchNavigationBadgesRefresh();
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
      } finally {
        endPluginMutation(plugin.id);
      }
    },
    [beginPluginMutation, client, endPluginMutation, refreshPlugins, setGlobalStatus, t],
  );

  const installPlugin = async (plugin: RegistryPluginRecord) => {
    beginPluginMutation(plugin.id);
    setPluginErrors((current) => {
      const next = { ...current };
      delete next[plugin.id];
      return next;
    });
    try {
      const { error } = await client
        .mutation(installPluginMutation, {
          input: { pluginId: plugin.id },
        })
        .toPromise();
      if (error) throw error;
      setGlobalStatus(t("status.pluginInstalled", { name: plugin.name }));
      await refreshPlugins();
      dispatchNavigationBadgesRefresh();
    } catch (error) {
      const message = formatPluginInstallError(plugin, error, t);
      setPluginErrors((current) => ({
        ...current,
        [plugin.id]: message,
      }));
      setGlobalStatus(message);
    } finally {
      endPluginMutation(plugin.id);
    }
  };

  const uninstallPlugin = (plugin: RegistryPluginRecord) => {
    setPendingUninstall(plugin);
  };

  const upgradePlugin = async (plugin: RegistryPluginRecord) => {
    beginPluginMutation(plugin.id);
    beginPluginUpgrade(plugin.id);
    try {
      const { error } = await client
        .mutation(upgradePluginMutation, {
          input: { pluginId: plugin.id },
        })
        .toPromise();
      if (error) throw error;
      setGlobalStatus(t("status.pluginUpgraded", { name: plugin.name, version: plugin.version }));
      await refreshPlugins();
      dispatchNavigationBadgesRefresh();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      endPluginUpgrade(plugin.id);
      endPluginMutation(plugin.id);
    }
  };

  const confirmUninstall = async () => {
    if (!pendingUninstall) return;
    const plugin = pendingUninstall;
    const isBuiltinOverride = plugin.builtin && plugin.sourceKind === "downloaded";
    beginPluginMutation(plugin.id);
    try {
      const { error } = await client
        .mutation(uninstallPluginMutation, {
          input: { pluginId: plugin.id },
        })
        .toPromise();
      if (error) throw error;
      setGlobalStatus(
        isBuiltinOverride
          ? t("status.pluginRevertedToBundled", { name: plugin.name })
          : t("status.pluginUninstalled", { name: plugin.name }),
      );
      await refreshPlugins();
      dispatchNavigationBadgesRefresh();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToDelete"));
    } finally {
      endPluginMutation(plugin.id);
      setPendingUninstall(null);
    }
  };

  const pendingUninstallIsBuiltinOverride =
    pendingUninstall?.builtin && pendingUninstall.sourceKind === "downloaded";

  return (
    <>
      <SettingsPluginsSection
        plugins={plugins}
        mutatingPluginIds={mutatingPluginIds}
        upgradingPluginIds={upgradingPluginIds}
        pluginErrors={pluginErrors}
        refreshing={refreshing}
        onRefreshRegistry={refreshRegistry}
        onTogglePlugin={togglePlugin}
        onInstallPlugin={installPlugin}
        onUninstallPlugin={uninstallPlugin}
        onUpgradePlugin={upgradePlugin}
      />
      <ConfirmDialog
        open={pendingUninstall !== null}
        title={
          pendingUninstallIsBuiltinOverride
            ? t("settings.pluginRevertToBundled")
            : t("settings.pluginUninstall")
        }
        description={
          pendingUninstall
            ? pendingUninstallIsBuiltinOverride
              ? t("settings.pluginRevertToBundledWarning", { name: pendingUninstall.name })
              : t("settings.pluginUninstallWarning", { name: pendingUninstall.name })
            : ""
        }
        confirmLabel={
          pendingUninstallIsBuiltinOverride
            ? t("settings.pluginRevert")
            : t("settings.pluginUninstall")
        }
        cancelLabel={t("label.cancel")}
        isBusy={pendingUninstall ? mutatingPluginIds.includes(pendingUninstall.id) : false}
        onConfirm={confirmUninstall}
        onCancel={() => setPendingUninstall(null)}
      />
    </>
  );
}
