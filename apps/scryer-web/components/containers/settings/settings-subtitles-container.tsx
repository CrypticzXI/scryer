import * as React from "react";
import { useClient } from "urql";
import { toast } from "sonner";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { SettingsSubtitleProvidersSection } from "@/components/views/settings/settings-subtitle-providers-section";
import { SettingsSubtitlesSection } from "@/components/views/settings/settings-subtitles-section";
import {
  subtitleProviderConfigsQuery,
  subtitleSettingsInitQuery,
  subtitleProviderTypesQuery,
} from "@/lib/graphql/queries";
import {
  createSubtitleProviderConfigMutation,
  deleteSubtitleProviderConfigMutation,
  testSubtitleProviderConnectionMutation,
  updateSubtitleProviderConfigMutation,
  updateSubtitleSettingsMutation,
} from "@/lib/graphql/mutations";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { runConnectionFeedback } from "@/lib/utils/connection-feedback";
import type {
  ConfigFieldDef,
  SubtitleProviderConfigRecord,
  SubtitleProviderDraft,
  SubtitleProviderTypeInfo,
  SubtitleProviderValidationResult,
  SubtitleSettings,
} from "@/lib/types";

const DEFAULTS: SubtitleSettings = {
  enabled: false,
  languages: [],
  autoDownloadOnImport: false,
  minimumScoreSeries: 240,
  minimumScoreMovie: 70,
  searchIntervalHours: 6,
  includeAiTranslated: false,
  includeMachineTranslated: false,
  syncEnabled: true,
  syncThresholdSeries: 90,
  syncThresholdMovie: 70,
  syncMaxOffsetSeconds: 60,
};

const DEFAULT_PROVIDER_DRAFT: SubtitleProviderDraft = {
  name: "",
  providerType: "",
  isEnabled: true,
  enabledFacets: [],
  configValues: {},
  persistedConfigValues: {},
  storedSecretKeys: [],
  configDirty: false,
};

const LEGACY_OPENSUBTITLES_CONFIG_KEYS = new Set([
  "include_ai_translated",
  "include_machine_translated",
]);

function sanitizePersistedConfigValues(
  providerType: string,
  configValues: Record<string, string>,
): Record<string, string> {
  if (providerType.trim().toLowerCase() !== "opensubtitles") {
    return configValues;
  }

  return Object.fromEntries(
    Object.entries(configValues).filter(
      ([key]) => !LEGACY_OPENSUBTITLES_CONFIG_KEYS.has(key),
    ),
  );
}

function buildDraftConfigValues(
  fields: ConfigFieldDef[],
  parsedConfigValues: Record<string, string>,
): Record<string, string> {
  if (fields.length === 0) {
    return { ...parsedConfigValues };
  }

  const nextValues: Record<string, string> = {};
  for (const field of fields) {
    if (field.valueSource === "host_binding") {
      continue;
    }
    nextValues[field.key] =
      parsedConfigValues[field.key] ??
      field.defaultValue ??
      (field.fieldType === "bool" ? "false" : "");
  }
  return nextValues;
}

function serializeProviderConfigJson(
  fields: ConfigFieldDef[],
  configValues: Record<string, string>,
  persistedConfigValues: Record<string, string>,
): string {
  const entries: Record<string, string> = {};

  if (fields.length === 0) {
    for (const [key, value] of Object.entries(configValues)) {
      if (value.trim() !== "") {
        entries[key] = value;
      }
    }
    return JSON.stringify(entries);
  }

  const fieldKeySet = new Set(fields.map((field) => field.key));
  for (const [key, value] of Object.entries(persistedConfigValues)) {
    if (!fieldKeySet.has(key) && value.trim() !== "") {
      entries[key] = value;
    }
  }

  for (const field of fields) {
    if (field.valueSource === "host_binding") {
      continue;
    }

    let nextValue = configValues[field.key] ?? "";
    const isSecretField =
      field.fieldType === "password" || field.fieldType === "secret";

    if (isSecretField && nextValue.trim() === "") {
      continue;
    }

    if (field.fieldType === "bool") {
      entries[field.key] =
        nextValue.trim() || field.defaultValue || "false";
      continue;
    }

    if (nextValue.trim() === "" && field.defaultValue) {
      nextValue = field.defaultValue;
    }

    if (nextValue.trim() !== "") {
      entries[field.key] = nextValue;
    }
  }

  return JSON.stringify(entries);
}

type SettingsSubtitlesContainerProps = {
  providerCatalogVersion?: number;
};

export function SettingsSubtitlesContainer({
  providerCatalogVersion = 0,
}: SettingsSubtitlesContainerProps) {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [settings, setSettings] = React.useState<SubtitleSettings>(DEFAULTS);
  const [saving, setSaving] = React.useState(false);
  const [loading, setLoading] = React.useState(true);
  const [providerTypes, setProviderTypes] = React.useState<SubtitleProviderTypeInfo[]>([]);
  const [providerConfigs, setProviderConfigs] = React.useState<SubtitleProviderConfigRecord[]>([]);
  const [providerDraft, setProviderDraft] = React.useState<SubtitleProviderDraft>(
    DEFAULT_PROVIDER_DRAFT,
  );
  const [editingProviderId, setEditingProviderId] = React.useState<string | null>(null);
  const [mutatingProviderId, setMutatingProviderId] = React.useState<string | null>(null);
  const [pendingDeleteProvider, setPendingDeleteProvider] =
    React.useState<SubtitleProviderConfigRecord | null>(null);
  const [isTestingProviderConnection, setIsTestingProviderConnection] =
    React.useState(false);
  const loadedRef = React.useRef(false);
  const providerCatalogVersionRef = React.useRef(providerCatalogVersion);

  const resetProviderDraft = React.useCallback(() => {
    setEditingProviderId(null);
    setProviderDraft({ ...DEFAULT_PROVIDER_DRAFT });
  }, []);

  const refreshProviderConfigs = React.useCallback(async () => {
    const { data, error } = await client
      .query(subtitleProviderConfigsQuery, {}, { requestPolicy: "network-only" })
      .toPromise();
    if (error) {
      throw error;
    }
    setProviderConfigs(
      (data?.subtitleProviderConfigs ?? []) as SubtitleProviderConfigRecord[],
    );
  }, [client]);

  const refreshProviderTypes = React.useCallback(async () => {
    const { data, error } = await client
      .query(subtitleProviderTypesQuery, {}, { requestPolicy: "network-only" })
      .toPromise();
    if (error) {
      throw error;
    }
    setProviderTypes(
      (data?.subtitleProviderTypes ?? []) as SubtitleProviderTypeInfo[],
    );
  }, [client]);

  React.useEffect(() => {
    if (editingProviderId || providerTypes.length === 0) {
      return;
    }

    setProviderDraft((previous) => {
      const configuredProvider =
        providerTypes.find(
          (providerType) => providerType.providerType === previous.providerType,
        ) ?? null;
      const nextProvider = configuredProvider ?? providerTypes[0] ?? null;
      if (!nextProvider) {
        return previous;
      }

      const shouldAutofillName =
        previous.name.trim().length === 0 ||
        previous.name === (configuredProvider?.name ?? previous.providerType);
      const nextProviderType = configuredProvider
        ? previous.providerType
        : nextProvider.providerType;
      const nextName = shouldAutofillName ? nextProvider.name : previous.name;

      if (
        nextProviderType === previous.providerType &&
        nextName === previous.name
      ) {
        return previous;
      }

      return {
        ...previous,
        providerType: nextProviderType,
        name: nextName,
      };
    });
  }, [editingProviderId, providerTypes]);

  React.useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const { data, error } = await client
          .query(subtitleSettingsInitQuery, {}, { requestPolicy: "network-only" })
          .toPromise();
        if (error) {
          throw error;
        }
        if (cancelled) {
          return;
        }

        const payload = data?.subtitleSettings;
        setSettings({
          ...DEFAULTS,
          ...(payload ?? {}),
        });
        setProviderTypes(
          (data?.subtitleProviderTypes ?? []) as SubtitleProviderTypeInfo[],
        );
        setProviderConfigs(
          (data?.subtitleProviderConfigs ?? []) as SubtitleProviderConfigRecord[],
        );
        loadedRef.current = true;
      } catch {
        // Keep defaults on failure.
      } finally {
        if (!cancelled) {
          setLoading(false);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [client]);

  React.useEffect(() => {
    if (providerCatalogVersion === providerCatalogVersionRef.current) {
      return;
    }

    providerCatalogVersionRef.current = providerCatalogVersion;
    void refreshProviderTypes().catch((error: unknown) => {
      const message =
        error instanceof Error ? error.message : t("status.failedToLoad");
      setGlobalStatus(message);
    });
  }, [providerCatalogVersion, refreshProviderTypes, setGlobalStatus, t]);

  React.useEffect(() => {
    if (!loadedRef.current) {
      return;
    }

    setSaving(true);
    client
      .mutation(updateSubtitleSettingsMutation, {
        input: {
          enabled: settings.enabled,
          languages: settings.languages.map((language) => ({
            code: language.code,
            hearingImpaired: language.hearingImpaired,
            forced: language.forced,
          })),
          autoDownloadOnImport: settings.autoDownloadOnImport,
          minimumScoreSeries: settings.minimumScoreSeries,
          minimumScoreMovie: settings.minimumScoreMovie,
          searchIntervalHours: settings.searchIntervalHours,
          includeAiTranslated: settings.includeAiTranslated,
          includeMachineTranslated: settings.includeMachineTranslated,
          syncEnabled: settings.syncEnabled,
          syncThresholdSeries: settings.syncThresholdSeries,
          syncThresholdMovie: settings.syncThresholdMovie,
          syncMaxOffsetSeconds: settings.syncMaxOffsetSeconds,
        },
      })
      .toPromise()
      .then(({ error }) => {
        if (error) {
          const message = error.message || t("status.failedToUpdate");
          setGlobalStatus(message);
          toast.error(message);
        }
      })
      .catch((error: unknown) => {
        const message =
          error instanceof Error ? error.message : t("status.failedToUpdate");
        setGlobalStatus(message);
        toast.error(message);
      })
      .finally(() => setSaving(false));
  }, [client, setGlobalStatus, settings, t]);

  const submitProvider = React.useCallback(
    async (event: React.FormEvent<HTMLFormElement>) => {
      event.preventDefault();

      const normalizedProviderType = providerDraft.providerType.trim().toLowerCase();
      const selectedProvider =
        providerTypes.find(
          (providerType) => providerType.providerType === normalizedProviderType,
        ) ?? null;
      const persistedConfigValues = sanitizePersistedConfigValues(
        normalizedProviderType,
        providerDraft.persistedConfigValues,
      );
      const payload = {
        name: providerDraft.name.trim(),
        providerType: normalizedProviderType,
        isEnabled: providerDraft.isEnabled,
        enabledFacets: providerDraft.enabledFacets,
      };
      const configJson = serializeProviderConfigJson(
        selectedProvider?.configFields ?? [],
        providerDraft.configValues,
        persistedConfigValues,
      );

      if (!payload.name || !payload.providerType) {
        setGlobalStatus(t("form.subtitleProviderValidation"));
        return;
      }

      setMutatingProviderId(editingProviderId || "new");
      try {
        if (editingProviderId) {
          const { error } = await client
            .mutation(updateSubtitleProviderConfigMutation, {
              input: {
                id: editingProviderId,
                name: payload.name,
                providerType: payload.providerType,
                configJson: providerDraft.configDirty ? configJson : undefined,
                enabledFacets: payload.enabledFacets,
                isEnabled: payload.isEnabled,
              },
            })
            .toPromise();
          if (error) {
            throw error;
          }
          setGlobalStatus(t("settings.subtitleProviderUpdated"));
        } else {
          const { error } = await client
            .mutation(createSubtitleProviderConfigMutation, {
              input: {
                name: payload.name,
                providerType: payload.providerType,
                configJson,
                enabledFacets: payload.enabledFacets,
                isEnabled: payload.isEnabled,
              },
            })
            .toPromise();
          if (error) {
            throw error;
          }
          setGlobalStatus(t("settings.subtitleProviderCreated"));
        }
        resetProviderDraft();
        await refreshProviderConfigs();
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToUpdate"),
        );
      } finally {
        setMutatingProviderId(null);
      }
    },
    [
      client,
      editingProviderId,
      providerDraft,
      providerTypes,
      refreshProviderConfigs,
      resetProviderDraft,
      setGlobalStatus,
      t,
    ],
  );

  const editProvider = React.useCallback(
    (provider: SubtitleProviderConfigRecord) => {
      const persistedConfigValues = sanitizePersistedConfigValues(
        provider.providerType,
        {},
      );
      const selectedProvider =
        providerTypes.find(
          (providerType) =>
            providerType.providerType === provider.providerType.toLowerCase(),
        ) ?? null;

      setEditingProviderId(provider.id);
      setProviderDraft({
        name: provider.name,
        providerType: provider.providerType,
        isEnabled: provider.isEnabled,
        enabledFacets: provider.enabledFacets ?? [],
        configValues: buildDraftConfigValues(
          selectedProvider?.configFields ?? [],
          persistedConfigValues,
        ),
        persistedConfigValues,
        storedSecretKeys: provider.storedSecretKeys ?? [],
        configDirty: false,
      });
      setGlobalStatus(t("status.editingSubtitleProvider", { name: provider.name }));
    },
    [providerTypes, setGlobalStatus, t],
  );

  const toggleProviderEnabled = React.useCallback(
    async (provider: SubtitleProviderConfigRecord) => {
      setMutatingProviderId(provider.id);
      try {
        const { error } = await client
          .mutation(updateSubtitleProviderConfigMutation, {
            input: {
              id: provider.id,
              isEnabled: !provider.isEnabled,
              enabledFacets: provider.enabledFacets ?? [],
            },
          })
          .toPromise();
        if (error) {
          throw error;
        }
        setGlobalStatus(t("settings.subtitleProviderUpdated"));
        await refreshProviderConfigs();
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToUpdate"),
        );
      } finally {
        setMutatingProviderId(null);
      }
    },
    [client, refreshProviderConfigs, setGlobalStatus, t],
  );

  const deleteProvider = React.useCallback(
    async (provider: SubtitleProviderConfigRecord) => {
      setPendingDeleteProvider(provider);
    },
    [],
  );

  const confirmDeleteProvider = React.useCallback(async () => {
    if (!pendingDeleteProvider) {
      return;
    }

    setMutatingProviderId(pendingDeleteProvider.id);
    try {
      const { error } = await client
        .mutation(deleteSubtitleProviderConfigMutation, {
          input: { id: pendingDeleteProvider.id },
        })
        .toPromise();
      if (error) {
        throw error;
      }
      setGlobalStatus(
        t("settings.subtitleProviderDeleted", {
          name: pendingDeleteProvider.name,
        }),
      );
      await refreshProviderConfigs();
      if (editingProviderId === pendingDeleteProvider.id) {
        resetProviderDraft();
      }
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToDelete"),
      );
    } finally {
      setMutatingProviderId(null);
      setPendingDeleteProvider(null);
    }
  }, [
    client,
    editingProviderId,
    pendingDeleteProvider,
    refreshProviderConfigs,
    resetProviderDraft,
    setGlobalStatus,
    t,
  ]);

  const testProviderConnection = React.useCallback(async () => {
    const normalizedProviderType = providerDraft.providerType.trim().toLowerCase();
    const selectedProvider =
      providerTypes.find(
        (providerType) => providerType.providerType === normalizedProviderType,
      ) ?? null;
    const persistedConfigValues = sanitizePersistedConfigValues(
      normalizedProviderType,
      providerDraft.persistedConfigValues,
    );
    if (!normalizedProviderType) {
      setGlobalStatus(t("form.subtitleProviderValidation"));
      return;
    }

    setIsTestingProviderConnection(true);
    try {
      await runConnectionFeedback({
        setGlobalStatus,
        startMessage: t("status.testingSubtitleProviderConnection"),
        successMessage: t("status.subtitleProviderConnectionTestPassed"),
        failureFallbackMessage: t("status.subtitleProviderConnectionTestFailed"),
        run: async () => {
          const { data, error } = await client
            .mutation(testSubtitleProviderConnectionMutation, {
              input: {
                id: editingProviderId ?? undefined,
                providerType: normalizedProviderType,
                configJson: serializeProviderConfigJson(
                  selectedProvider?.configFields ?? [],
                  providerDraft.configValues,
                  persistedConfigValues,
                ),
              },
            })
            .toPromise();
          if (error) {
            throw error;
          }

          const validation = data?.testSubtitleProviderConnection as
            | SubtitleProviderValidationResult
            | undefined;
          const success =
            validation?.status === "valid" || validation?.status === "ok";
          if (!success) {
            throw new Error(
              validation?.message ||
                t("status.subtitleProviderConnectionTestFailed"),
            );
          }

          return (
            validation?.message ||
            t("status.subtitleProviderConnectionTestPassed")
          );
        },
      });
    } catch {
      // Connection feedback is already surfaced through the shared helper.
    } finally {
      setIsTestingProviderConnection(false);
    }
  }, [client, editingProviderId, providerDraft, providerTypes, setGlobalStatus, t]);

  return (
    <>
      <div className="space-y-6">
        <SettingsSubtitlesSection
          settings={settings}
          setSettings={setSettings}
          saving={saving}
          loading={loading}
        />
        {!loading ? (
          <SettingsSubtitleProvidersSection
            editingProviderId={editingProviderId}
            providerDraft={providerDraft}
            setProviderDraft={setProviderDraft}
            submitProvider={submitProvider}
            mutatingProviderId={mutatingProviderId}
            resetProviderDraft={resetProviderDraft}
            providerConfigs={providerConfigs}
            editProvider={editProvider}
            toggleProviderEnabled={toggleProviderEnabled}
            deleteProvider={deleteProvider}
          providerTypes={providerTypes}
          testProviderConnection={testProviderConnection}
          isTestingConnection={isTestingProviderConnection}
        />
      ) : null}
      </div>
      <ConfirmDialog
        open={pendingDeleteProvider !== null}
        title={t("label.delete")}
        description={
          pendingDeleteProvider
            ? t("status.deletingSubtitleProvider", {
                name: pendingDeleteProvider.name,
              })
            : ""
        }
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        isBusy={mutatingProviderId !== null}
        onConfirm={confirmDeleteProvider}
        onCancel={() => setPendingDeleteProvider(null)}
      />
    </>
  );
}
