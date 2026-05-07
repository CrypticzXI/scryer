import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { Dispatch, SetStateAction } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";
import { toast } from "sonner";
import { useClient } from "urql";

import {
  pluginsQuery,
  pluginInstallProgressSubscription,
  qualityProfilesInitQuery,
  setupWizardProviderTypesInitQuery,
} from "@/lib/graphql/queries";
import {
  saveQualityProfileSettingsMutation,
  updateLibraryPathsMutation,
  createDownloadClientMutation,
  testDownloadClientConnectionMutation,
  createIndexerMutation,
  testIndexerConnectionMutation,
  completeSetupMutation,
  previewExternalImportMutation,
  executeExternalImportMutation,
  beginInstallPluginMutation,
  refreshPluginCatalogMutation,
  uninstallPluginMutation,
  scanLibraryMutation,
} from "@/lib/graphql/mutations";
import { wsClient } from "@/lib/graphql/ws-client";
import { DEFAULT_DOWNLOAD_CLIENT_DRAFT, DEFAULT_PORT_FOR_CLIENT_TYPE } from "@/lib/constants/download-clients";
import {
  buildDownloadClientConfigJson,
  buildDownloadClientTypeOptions,
  ensureDownloadClientTypeOption,
  normalizeDownloadClientType,
} from "@/lib/utils/download-clients";
import {
  qualityProfileSettingsToEntries,
  qualityProfileEntryToMutationInput,
} from "@/lib/utils/quality-profiles";
import type { DownloadClientDraft, DownloadClientTypeOption } from "@/lib/types/download-clients";
import type { FacetQualityPrefs, ViewCategoryId } from "@/lib/types/quality-profiles";
import type { ExternalImportPreview, ExternalImportResult } from "@/lib/types/external-import";
import type { ConfigFieldDef, ProviderTypeInfo } from "@/lib/types";

import { SetupProgressBar } from "./setup-progress-bar";
import { SetupWelcomeView } from "./setup-welcome-view";
import { SetupPersonaView } from "./setup-persona-view";
import { SetupMediaPathsView } from "./setup-media-paths-view";
import { SetupDownloadClientView } from "./setup-download-client-view";
import { SetupIndexerView } from "./setup-indexer-view";
import { SetupSummaryView } from "./setup-summary-view";
import { SetupImportConnectView } from "./setup-import-connect-view";
import { SetupImportReviewView } from "./setup-import-review-view";
import { SetupPluginsView } from "./setup-plugins-view";
import type {
  PluginInstallProgressRecord,
  RegistryPluginRecord,
} from "@/components/views/settings/settings-plugins-section";

type SetupIndexerProviderOption = {
  value: string;
  label: string;
  defaultBaseUrl?: string;
  configFields: ConfigFieldDef[];
};

const FALLBACK_PROVIDER_OPTIONS: SetupIndexerProviderOption[] = [];

function setupIndexerConfigFields(fields: ConfigFieldDef[]) {
  return fields.filter((field) => field.valueSource !== "host_binding");
}

function buildSetupIndexerConfigValues(
  fields: ConfigFieldDef[],
): Record<string, string> {
  const values: Record<string, string> = {};
  for (const field of setupIndexerConfigFields(fields)) {
    values[field.key] =
      field.defaultValue ?? (field.fieldType === "bool" ? "false" : "");
  }
  return values;
}

function serializeSetupIndexerConfigJson(
  fields: ConfigFieldDef[],
  values: Record<string, string>,
): string | undefined {
  const entries: Record<string, string> = {};
  const fieldKeySet = new Set(fields.map((field) => field.key));

  for (const [key, value] of Object.entries(values)) {
    if (!fieldKeySet.has(key) && value.trim() !== "") {
      entries[key] = value;
    }
  }

  for (const field of setupIndexerConfigFields(fields)) {
    let value =
      values[field.key] ??
      field.defaultValue ??
      (field.fieldType === "bool" ? "false" : "");
    if (field.fieldType === "bool") {
      entries[field.key] = value.trim() || field.defaultValue || "false";
      continue;
    }
    if (value.trim() === "" && field.defaultValue) {
      value = field.defaultValue;
    }
    if (value.trim() !== "") {
      entries[field.key] = value;
    }
  }

  return Object.keys(entries).length > 0 ? JSON.stringify(entries) : undefined;
}

function findMissingSetupIndexerField(
  fields: ConfigFieldDef[],
  values: Record<string, string>,
): ConfigFieldDef | null {
  for (const field of setupIndexerConfigFields(fields)) {
    if (!field.required) {
      continue;
    }
    const value =
      values[field.key] ??
      field.defaultValue ??
      (field.fieldType === "bool" ? "false" : "");
    if (field.fieldType !== "bool" && value.trim() === "") {
      return field;
    }
  }
  return null;
}

type PluginInstallProgressSubscriptionResult = {
  data?: {
    pluginInstallProgress?: PluginInstallProgressRecord;
  };
};

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

function extractPluginMutationErrorCode(error: unknown): string | null {
  if (
    error
    && typeof error === "object"
    && "graphQLErrors" in error
    && Array.isArray((error as { graphQLErrors?: unknown[] }).graphQLErrors)
  ) {
    const graphQLErrors = (error as {
      graphQLErrors?: Array<{ extensions?: { code?: unknown } }>;
    }).graphQLErrors;
    const code = graphQLErrors?.find(
      (entry) => typeof entry.extensions?.code === "string",
    )?.extensions?.code;
    return typeof code === "string" ? code : null;
  }

  return null;
}

function formatPluginInstallError(
  plugin: RegistryPluginRecord,
  error: unknown,
  t: SetupWizardContainerProps["t"],
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
    if (extractPluginMutationErrorCode(error) === "PLUGIN_INSTALL_IN_PROGRESS") {
      return t("status.pluginInstallAlreadyInProgress", { name: plugin.name });
    }
    return t("status.pluginInstallFailedWithReason", {
      name: plugin.name,
      reason: normalized,
    });
  }

  return t("status.failedToUpdate");
}

interface SetupWizardContainerProps {
  t: (
    key: string,
    values?: Record<string, string | number | boolean | null | undefined>,
  ) => string;
  isReentry?: boolean;
}

export function SetupWizardContainer({ t, isReentry }: SetupWizardContainerProps) {
  const client = useClient();
  const navigate = useNavigate();

  // ── Wizard path + step (URL-driven for browser back/forward) ──────
  const [searchParams, setSearchParams] = useSearchParams();
  const wizardPath: "fresh" | "import" = searchParams.get("path") === "import" ? "import" : "fresh";
  const currentStep = parseInt(searchParams.get("step") || "0", 10);

  const goToStep = useCallback(
    (step: number, path?: "fresh" | "import") => {
      const p = path ?? wizardPath;
      if (step === 0) {
        setSearchParams({});
      } else {
        setSearchParams({ path: p, step: String(step) });
      }
    },
    [wizardPath, setSearchParams],
  );

  // ── Step 1 (fresh) / Step 3 (import): Quality Preferences ─────────
  const [facetPrefs, setFacetPrefs] = useState<Record<ViewCategoryId, FacetQualityPrefs>>({
    movie:  { quality: "4k",    persona: "Balanced" },
    series: { quality: "4k",    persona: "Balanced" },
    anime:  { quality: "1080p", persona: "Balanced" },
  });
  const [personaSaving, setPersonaSaving] = useState(false);

  // ── Step 2 (fresh): Media Paths ─────────────────────────────────────
  const [moviesPath, setMoviesPath] = useState("/data/movies");
  const [seriesPath, setSeriesPath] = useState("/data/series");
  const [animePath, setAnimePath] = useState("");
  const [mediaPathsSaving, setMediaPathsSaving] = useState(false);
  const [mediaPathsError, setMediaPathsError] = useState<string | null>(null);

  // ── Step 4 (fresh): Download Client ─────────────────────────────────
  const [dcDraft, setDcDraft] = useState<DownloadClientDraft>({ ...DEFAULT_DOWNLOAD_CLIENT_DRAFT });
  const [dcTypeOptions, setDcTypeOptions] = useState<DownloadClientTypeOption[]>(
    () => buildDownloadClientTypeOptions([]),
  );
  const [dcTesting, setDcTesting] = useState(false);
  const [dcTestResult, setDcTestResult] = useState<"success" | "failed" | null>(null);
  const [dcSaving, setDcSaving] = useState(false);
  const [dcSaved, setDcSaved] = useState(false);
  const [dcError, setDcError] = useState<string | null>(null);

  // ── Step 5 (fresh): Indexer ─────────────────────────────────────────
  const [idxName, setIdxName] = useState("");
  const [idxProviderType, setIdxProviderType] = useState("");
  const [idxConfigValues, setIdxConfigValues] = useState<Record<string, string>>({});
  const [idxProviderOptions, setIdxProviderOptions] =
    useState<SetupIndexerProviderOption[]>([]);
  const [idxTesting, setIdxTesting] = useState(false);
  const [idxTestResult, setIdxTestResult] = useState<"success" | "failed" | null>(null);
  const [idxSaving, setIdxSaving] = useState(false);
  const [idxSaved, setIdxSaved] = useState(false);
  const [idxError, setIdxError] = useState<string | null>(null);

  // ── Step 3 (fresh): Plugins ────────────────────────────────────────
  const [plugins, setPlugins] = useState<RegistryPluginRecord[]>([]);
  const [pluginsLoading, setPluginsLoading] = useState(true);
  const [pluginsRefreshing, setPluginsRefreshing] = useState(false);
  const [mutatingPluginIds, setMutatingPluginIds] = useState<string[]>([]);
  const [pluginProgress, setPluginProgress] = useState<Record<string, PluginInstallProgressRecord>>({});
  const [pluginErrors, setPluginErrors] = useState<Record<string, string>>({});
  const [pluginsError, setPluginsError] = useState<string | null>(null);
  const installProgressSubscriptionsRef = useRef(new Map<string, () => void>());

  // ── Import: Connect ─────────────────────────────────────────────────
  const [sonarrUrl, setSonarrUrl] = useState("");
  const [sonarrApiKey, setSonarrApiKey] = useState("");
  const [radarrUrl, setRadarrUrl] = useState("");
  const [radarrApiKey, setRadarrApiKey] = useState("");
  const [importConnecting, setImportConnecting] = useState(false);
  const [importConnectError, setImportConnectError] = useState<string | null>(null);

  // ── Import: Preview / Review ────────────────────────────────────────
  const [importPreview, setImportPreview] = useState<ExternalImportPreview | null>(null);
  const [selectedMoviesPaths, setSelectedMoviesPaths] = useState<string[]>([]);
  const [selectedSeriesPaths, setSelectedSeriesPaths] = useState<string[]>([]);
  const [customMoviesPaths, setCustomMoviesPaths] = useState<string[]>([]);
  const [customSeriesPaths, setCustomSeriesPaths] = useState<string[]>([]);
  const [selectedDcKeys, setSelectedDcKeys] = useState<Set<string>>(new Set());
  const [selectedIdxKeys, setSelectedIdxKeys] = useState<Set<string>>(new Set());
  // User-supplied API keys for clients whose keys were masked by Sonarr/Radarr.
  const [dcApiKeyOverrides, setDcApiKeyOverrides] = useState<Map<string, string>>(new Map());
  const [selectedAnimePaths, setSelectedAnimePaths] = useState<string[]>([]);
  const [customAnimePaths, setCustomAnimePaths] = useState<string[]>([]);
  const [importExecuting, setImportExecuting] = useState(false);
  const [importExecuteError, setImportExecuteError] = useState<string | null>(null);
  const [importResult, setImportResult] = useState<ExternalImportResult | null>(null);

  // ── Summary / Finish ────────────────────────────────────────────────
  const [finishingAction, setFinishingAction] = useState<
    "finish" | "importOnly" | "importAndScan" | null
  >(null);
  const finishing = finishingAction !== null;

  const beginPluginMutation = useCallback((pluginId: string) => {
    setMutatingPluginIds((current) => (
      current.includes(pluginId) ? current : [...current, pluginId]
    ));
  }, []);

  const endPluginMutation = useCallback((pluginId: string) => {
    setMutatingPluginIds((current) => current.filter((id) => id !== pluginId));
  }, []);

  const clearPluginProgress = useCallback((pluginId: string) => {
    setPluginProgress((current) => {
      if (!(pluginId in current)) {
        return current;
      }
      const next = { ...current };
      delete next[pluginId];
      return next;
    });
  }, []);

  const stopPluginInstallProgressSubscription = useCallback((pluginId: string) => {
    const unsubscribe = installProgressSubscriptionsRef.current.get(pluginId);
    if (unsubscribe) {
      unsubscribe();
      installProgressSubscriptionsRef.current.delete(pluginId);
    }
  }, []);

  const refreshProviderOptions = useCallback(async () => {
    try {
      const { data, error } = await client.query(setupWizardProviderTypesInitQuery, {}).toPromise();
      if (error && !data?.downloadClientProviderTypes && !data?.indexerProviderTypes) throw error;

      setDcTypeOptions(
        buildDownloadClientTypeOptions(
          (data?.downloadClientProviderTypes as ProviderTypeInfo[] | undefined) ?? [],
        ),
      );

      if (data?.indexerProviderTypes?.length) {
        setIdxProviderOptions(
          data.indexerProviderTypes.map(
            (provider: ProviderTypeInfo) => ({
              value: provider.providerType,
              label: provider.name,
              defaultBaseUrl: provider.defaultBaseUrl || undefined,
              configFields: provider.configFields ?? [],
            }),
          ),
        );
      } else {
        setIdxProviderOptions(FALLBACK_PROVIDER_OPTIONS);
      }
    } catch {
      setDcTypeOptions(buildDownloadClientTypeOptions([]));
      setIdxProviderOptions(FALLBACK_PROVIDER_OPTIONS);
    }
  }, [client]);

  const loadPlugins = useCallback(
    async (refreshIfEmpty = false) => {
      const { data, error } = await client.query(pluginsQuery, {}).toPromise();
      if (error) throw error;

      const nextPlugins = (data?.plugins ?? []) as RegistryPluginRecord[];
      if (nextPlugins.length > 0 || !refreshIfEmpty) {
        setPlugins(nextPlugins);
        return nextPlugins;
      }

      const { data: refreshData, error: refreshError } = await client
        .mutation(refreshPluginCatalogMutation, {})
        .toPromise();
      if (refreshError) throw refreshError;

      const refreshedPlugins = (refreshData?.refreshPluginCatalog ?? []) as RegistryPluginRecord[];
      setPlugins(refreshedPlugins);
      return refreshedPlugins;
    },
    [client],
  );

  useEffect(() => {
    void (async () => {
      setPluginsLoading(true);
      setPluginsError(null);
      try {
        await Promise.all([refreshProviderOptions(), loadPlugins(true)]);
      } catch (error) {
        setPluginsError(error instanceof Error ? error.message : t("status.failedToLoad"));
      } finally {
        setPluginsLoading(false);
      }
    })();
  }, [loadPlugins, refreshProviderOptions, t]);

  useEffect(() => {
    setDcDraft((prev) => {
      const normalizedClientType = normalizeDownloadClientType(prev.clientType);
      if (dcTypeOptions.some((option) => option.value === normalizedClientType)) {
        return prev;
      }

      return {
        ...prev,
        clientType: dcTypeOptions[0]?.value ?? DEFAULT_DOWNLOAD_CLIENT_DRAFT.clientType,
      };
    });
  }, [dcTypeOptions]);

  useEffect(() => {
    if (idxProviderOptions.some((option) => option.value === idxProviderType)) {
      return;
    }
    const firstProvider = idxProviderOptions[0];
    if (firstProvider?.value) {
      setIdxProviderType(firstProvider.value);
      setIdxConfigValues(buildSetupIndexerConfigValues(firstProvider.configFields));
      setIdxName((current) => current || firstProvider.label);
    }
  }, [idxProviderOptions, idxProviderType]);

  useEffect(() => {
    const subscriptions = installProgressSubscriptionsRef.current;
    return () => {
      for (const unsubscribe of subscriptions.values()) {
        unsubscribe();
      }
      subscriptions.clear();
    };
  }, []);

  const availableDcTypeOptions = ensureDownloadClientTypeOption(dcTypeOptions, dcDraft.clientType);
  const selectedIdxProvider = useMemo(
    () => idxProviderOptions.find((option) => option.value === idxProviderType) ?? null,
    [idxProviderOptions, idxProviderType],
  );
  const selectedIdxProviderFields = useMemo(
    () => selectedIdxProvider?.configFields ?? [],
    [selectedIdxProvider],
  );

  const resetIndexerSavedState = useCallback(() => {
    setIdxSaved(false);
    setIdxTestResult(null);
    setIdxError(null);
  }, []);

  const handleIdxNameChange = useCallback(
    (value: string) => {
      setIdxName(value);
      resetIndexerSavedState();
    },
    [resetIndexerSavedState],
  );

  const handleIdxProviderTypeChange = useCallback(
    (nextProviderType: string) => {
      const nextProvider =
        idxProviderOptions.find((option) => option.value === nextProviderType) ??
        null;
      setIdxProviderType(nextProviderType);
      setIdxConfigValues(
        buildSetupIndexerConfigValues(nextProvider?.configFields ?? []),
      );
      setIdxName((current) => current || nextProvider?.label || "");
      resetIndexerSavedState();
    },
    [idxProviderOptions, resetIndexerSavedState],
  );

  const handleIdxConfigValueChange = useCallback(
    (key: string, value: string) => {
      setIdxConfigValues((current) => ({ ...current, [key]: value }));
      resetIndexerSavedState();
    },
    [resetIndexerSavedState],
  );

  const buildIndexerConfigJson = useCallback(() => {
    if (!idxProviderType) {
      setIdxError(t("form.providerTypePlaceholder"));
      return null;
    }

    const missingField = findMissingSetupIndexerField(
      selectedIdxProviderFields,
      idxConfigValues,
    );
    if (missingField) {
      setIdxError(`${missingField.label}: ${t("setup.required")}`);
      return null;
    }

    return serializeSetupIndexerConfigJson(
      selectedIdxProviderFields,
      idxConfigValues,
    );
  }, [idxConfigValues, idxProviderType, selectedIdxProviderFields, t]);

  const refreshPluginsRegistry = useCallback(async () => {
    setPluginsRefreshing(true);
    setPluginsError(null);
    try {
      const { data, error } = await client
        .mutation(refreshPluginCatalogMutation, {})
        .toPromise();
      if (error) throw error;

      setPlugins((data?.refreshPluginCatalog ?? []) as RegistryPluginRecord[]);
    } catch (error) {
      setPluginsError(error instanceof Error ? error.message : t("status.failedToLoad"));
    } finally {
      setPluginsRefreshing(false);
    }
  }, [client, t]);

  const beginLivePluginProgress = useCallback(
    (
      plugin: RegistryPluginRecord,
      initialSnapshot: PluginInstallProgressRecord,
    ) => {
      stopPluginInstallProgressSubscription(plugin.id);
      setPluginProgress((current) => ({
        ...current,
        [plugin.id]: initialSnapshot,
      }));
      const unsubscribe = wsClient.subscribe(
        {
          query: pluginInstallProgressSubscription,
          variables: { pluginId: plugin.id },
        },
        {
          next: (result: PluginInstallProgressSubscriptionResult) => {
            const snapshot = result.data?.pluginInstallProgress;
            if (!snapshot) {
              return;
            }
            setPluginProgress((current) => ({
              ...current,
              [plugin.id]: snapshot,
            }));

            if (snapshot.state === "succeeded" || snapshot.state === "failed") {
              stopPluginInstallProgressSubscription(plugin.id);
              void (async () => {
                try {
                  if (snapshot.state === "succeeded") {
                    setPluginErrors((current) => {
                      const next = { ...current };
                      delete next[plugin.id];
                      return next;
                    });
                    await Promise.all([loadPlugins(false), refreshProviderOptions()]);
                  } else {
                    setPluginErrors((current) => ({
                      ...current,
                      [plugin.id]: formatPluginInstallError(
                        plugin,
                        new Error(snapshot.error ?? snapshot.label),
                        t,
                      ),
                    }));
                  }
                } catch (error) {
                  setPluginsError(error instanceof Error ? error.message : t("status.failedToLoad"));
                } finally {
                  clearPluginProgress(plugin.id);
                  endPluginMutation(plugin.id);
                }
              })();
            }
          },
          error: (error) => {
            stopPluginInstallProgressSubscription(plugin.id);
            clearPluginProgress(plugin.id);
            endPluginMutation(plugin.id);
            setPluginErrors((current) => ({
              ...current,
              [plugin.id]: formatPluginInstallError(plugin, error, t),
            }));
          },
          complete: () => {
            installProgressSubscriptionsRef.current.delete(plugin.id);
          },
        },
      );
      installProgressSubscriptionsRef.current.set(plugin.id, unsubscribe);
    },
    [
      clearPluginProgress,
      endPluginMutation,
      loadPlugins,
      refreshProviderOptions,
      stopPluginInstallProgressSubscription,
      t,
    ],
  );

  const installPlugin = useCallback(
    async (plugin: RegistryPluginRecord) => {
      beginPluginMutation(plugin.id);
      setPluginErrors((current) => {
        const next = { ...current };
        delete next[plugin.id];
        return next;
      });
      setPluginsError(null);
      try {
        const { data, error } = await client
          .mutation(beginInstallPluginMutation, {
            input: { pluginId: plugin.id },
          })
          .toPromise();
        if (error) throw error;
        const snapshot = data?.beginInstallPlugin;
        if (!snapshot) {
          throw new Error("plugin install did not return progress");
        }
        beginLivePluginProgress(plugin, snapshot);
      } catch (error) {
        setPluginErrors((current) => ({
          ...current,
          [plugin.id]: formatPluginInstallError(plugin, error, t),
        }));
        endPluginMutation(plugin.id);
      }
    },
    [beginLivePluginProgress, beginPluginMutation, client, endPluginMutation, t],
  );

  const uninstallPlugin = useCallback(
    async (plugin: RegistryPluginRecord) => {
      beginPluginMutation(plugin.id);
      setPluginErrors((current) => {
        const next = { ...current };
        delete next[plugin.id];
        return next;
      });
      setPluginsError(null);
      try {
        const { error } = await client
          .mutation(uninstallPluginMutation, {
            input: { pluginId: plugin.id },
          })
          .toPromise();
        if (error) throw error;

        await Promise.all([loadPlugins(false), refreshProviderOptions()]);
      } catch (error) {
        setPluginErrors((current) => ({
          ...current,
          [plugin.id]: extractPluginMutationErrorMessage(error) ?? t("status.failedToDelete"),
        }));
      } finally {
        endPluginMutation(plugin.id);
      }
    },
    [beginPluginMutation, client, endPluginMutation, loadPlugins, refreshProviderOptions, t],
  );

  // ── Step labels per path ────────────────────────────────────────────
  const stepLabels =
    wizardPath === "import"
      ? [t("setup.stepConnect"), t("setup.stepReview"), t("setup.stepPersona"), t("setup.stepSummary")]
      : [t("setup.stepPersona"), t("setup.stepMediaPaths"), t("setup.stepPlugins"), t("setup.stepDownloadClient"), t("setup.stepIndexer"), t("setup.stepSummary")];

  // ── Quality preferences save (per-facet) ────────────────────────────
  const saveFacetQualityPrefs = useCallback(
    async (nextStep: number) => {
      setPersonaSaving(true);
      try {
        const { data } = await client.query(qualityProfilesInitQuery, {}).toPromise();
        const existingProfiles = qualityProfileSettingsToEntries(data?.qualityProfileSettings);

        // Build per-facet profiles from templates
        const WIZARD_FACETS: { facet: ViewCategoryId; name: string }[] = [
          { facet: "movie", name: "Movies" },
          { facet: "series", name: "Series" },
          { facet: "anime", name: "Anime" },
        ];
        const wizardProfileIds = WIZARD_FACETS.map((f) => `wizard-${f.facet}`);
        const builtinProfileIds = ["4k", "1080p"];
        const keptProfiles = existingProfiles.filter(
          (p) => !wizardProfileIds.includes(p.id) && !builtinProfileIds.includes(p.id),
        );

        for (const { facet, name } of WIZARD_FACETS) {
          const prefs = facetPrefs[facet];
          const template = existingProfiles.find((p) => p.id === prefs.quality);
          if (template) {
            const profileName = `${name} (${prefs.quality === "4k" ? "4K" : "1080P"})`;
            keptProfiles.push({
              id: `wizard-${facet}`,
              name: profileName,
              criteria: { ...template.criteria },
            });
          }
        }

        await client
          .mutation(saveQualityProfileSettingsMutation, {
            input: {
              profiles: keptProfiles.map(qualityProfileEntryToMutationInput),
              globalProfileId: null,
              globalScoringPersona: null,
              categorySelections: WIZARD_FACETS.map(({ facet }) => ({
                scope: facet,
                profileId: `wizard-${facet}`,
                inheritGlobal: false,
              })),
              categoryPersonaSelections: WIZARD_FACETS.map(({ facet }) => ({
                scope: facet,
                persona: facetPrefs[facet].persona,
                inheritGlobal: false,
              })),
              replaceExisting: true,
            },
          })
          .toPromise();

        goToStep(nextStep);
      } catch (err) {
        console.warn("Failed to save quality preferences, continuing", err);
        goToStep(nextStep);
      } finally {
        setPersonaSaving(false);
      }
    },
    [client, facetPrefs, goToStep],
  );

  // ── Media paths save ────────────────────────────────────────────────
  const saveMediaPaths = useCallback(async () => {
    setMediaPathsSaving(true);
    setMediaPathsError(null);
    try {
      const trimmedMovies = moviesPath.trim();
      const trimmedSeries = seriesPath.trim();
      const trimmedAnime = animePath.trim();
      if (!trimmedMovies && !trimmedSeries && !trimmedAnime) {
        goToStep(3);
        return;
      }
      const { error } = await client
        .mutation(updateLibraryPathsMutation, {
          input: {
            moviePath: trimmedMovies,
            seriesPath: trimmedSeries,
            animePath: trimmedAnime.length > 0 ? trimmedAnime : null,
          },
        })
        .toPromise();
      if (error) throw error;
      goToStep(3);
    } catch (err) {
      setMediaPathsError(err instanceof Error ? err.message : "Failed to save");
    } finally {
      setMediaPathsSaving(false);
    }
  }, [client, moviesPath, seriesPath, animePath, goToStep]);

  // ── Download client test ────────────────────────────────────────────
  const testDownloadClient = useCallback(async () => {
    setDcTesting(true);
    setDcTestResult(null);
    setDcError(null);
    try {
      const { data, error } = await client
        .mutation(testDownloadClientConnectionMutation, {
          input: {
            clientType: dcDraft.clientType,
            configJson: buildDownloadClientConfigJson(dcDraft),
          },
        })
        .toPromise();
      if (error) throw error;
      if (data?.testDownloadClientConnection) {
        setDcTestResult("success");
      } else {
        setDcTestResult("failed");
      }
    } catch {
      setDcTestResult("failed");
    } finally {
      setDcTesting(false);
    }
  }, [client, dcDraft]);

  // ── Download client save ────────────────────────────────────────────
  const saveDownloadClient = useCallback(async () => {
    setDcSaving(true);
    setDcError(null);
    try {
      const { error } = await client
        .mutation(createDownloadClientMutation, {
          input: {
            name: dcDraft.name.trim(),
            clientType: dcDraft.clientType,
            configJson: buildDownloadClientConfigJson(dcDraft),
            isEnabled: true,
          },
        })
        .toPromise();
      if (error) throw error;
      setDcSaved(true);
    } catch (err) {
      setDcError(err instanceof Error ? err.message : "Failed to save");
    } finally {
      setDcSaving(false);
    }
  }, [client, dcDraft]);

  const handleDcTestAndSave = useCallback(async () => {
    setDcTesting(true);
    setDcTestResult(null);
    setDcError(null);
    try {
      const { data, error } = await client
        .mutation(testDownloadClientConnectionMutation, {
          input: {
            clientType: dcDraft.clientType,
            configJson: buildDownloadClientConfigJson(dcDraft),
          },
        })
        .toPromise();
      if (error) throw error;
      if (data?.testDownloadClientConnection) {
        setDcTestResult("success");
        setDcTesting(false);
        await saveDownloadClient();
      } else {
        setDcTestResult("failed");
        setDcTesting(false);
      }
    } catch {
      setDcTestResult("failed");
      setDcTesting(false);
    }
  }, [client, dcDraft, saveDownloadClient]);

  // ── Indexer test ────────────────────────────────────────────────────
  const testIndexer = useCallback(async () => {
    setIdxTesting(true);
    setIdxTestResult(null);
    setIdxError(null);
    const configJson = buildIndexerConfigJson();
    if (configJson === null) {
      setIdxTesting(false);
      return;
    }
    try {
      const { data, error } = await client
        .mutation(testIndexerConnectionMutation, {
          input: {
            providerType: idxProviderType,
            configJson,
          },
        })
        .toPromise();
      if (error) throw error;
      if (data?.testIndexerConnection) {
        setIdxTestResult("success");
      } else {
        setIdxTestResult("failed");
      }
    } catch {
      setIdxTestResult("failed");
    } finally {
      setIdxTesting(false);
    }
  }, [buildIndexerConfigJson, client, idxProviderType]);

  // ── Indexer save ────────────────────────────────────────────────────
  const saveIndexer = useCallback(async () => {
    setIdxSaving(true);
    setIdxError(null);
    const configJson = buildIndexerConfigJson();
    if (configJson === null) {
      setIdxSaving(false);
      return;
    }
    try {
      const { error } = await client
        .mutation(createIndexerMutation, {
          input: {
            name: idxName.trim(),
            providerType: idxProviderType,
            configJson,
            isEnabled: true,
            enableInteractiveSearch: true,
            enableAutoSearch: true,
          },
        })
        .toPromise();
      if (error) throw error;
      setIdxSaved(true);
    } catch (err) {
      setIdxError(err instanceof Error ? err.message : "Failed to save");
    } finally {
      setIdxSaving(false);
    }
  }, [buildIndexerConfigJson, client, idxName, idxProviderType]);

  const handleIdxTestAndSave = useCallback(async () => {
    setIdxTesting(true);
    setIdxTestResult(null);
    setIdxError(null);
    const configJson = buildIndexerConfigJson();
    if (configJson === null) {
      setIdxTesting(false);
      return;
    }
    try {
      const { data, error } = await client
        .mutation(testIndexerConnectionMutation, {
          input: {
            providerType: idxProviderType,
            configJson,
          },
        })
        .toPromise();
      if (error) throw error;
      if (data?.testIndexerConnection) {
        setIdxTestResult("success");
        setIdxTesting(false);
        await saveIndexer();
      } else {
        setIdxTestResult("failed");
        setIdxTesting(false);
      }
    } catch {
      setIdxTestResult("failed");
      setIdxTesting(false);
    }
  }, [buildIndexerConfigJson, client, idxProviderType, saveIndexer]);

  // ── Import: Connect & Scan ──────────────────────────────────────────
  const handleImportConnect = useCallback(async () => {
    setImportConnecting(true);
    setImportConnectError(null);
    try {
      const sonarr =
        sonarrUrl.trim() && sonarrApiKey.trim()
          ? { baseUrl: sonarrUrl.trim(), apiKey: sonarrApiKey.trim() }
          : undefined;
      const radarr =
        radarrUrl.trim() && radarrApiKey.trim()
          ? { baseUrl: radarrUrl.trim(), apiKey: radarrApiKey.trim() }
          : undefined;

      const { data, error } = await client
        .mutation(previewExternalImportMutation, {
          input: { sonarr: sonarr ?? null, radarr: radarr ?? null },
        })
        .toPromise();
      if (error) throw error;

      const preview: ExternalImportPreview = data.previewExternalImport;

      if (!preview.sonarrConnected && sonarr) {
        setImportConnectError("Could not connect to Sonarr. Check the URL and API key.");
        setImportConnecting(false);
        return;
      }
      if (!preview.radarrConnected && radarr) {
        setImportConnectError("Could not connect to Radarr. Check the URL and API key.");
        setImportConnecting(false);
        return;
      }

      setImportPreview(preview);

      // Auto-select all supported items
      const dcKeys = new Set<string>();
      for (const dc of preview.downloadClients) {
        if (dc.supported) dcKeys.add(dc.dedupKey);
      }
      setSelectedDcKeys(dcKeys);

      const idxKeys = new Set<string>();
      for (const idx of preview.indexers) {
        if (idx.supported) idxKeys.add(idx.dedupKey);
      }
      setSelectedIdxKeys(idxKeys);

      // Auto-select all Radarr roots for movies.
      const radarrFolders = preview.rootFolders.filter((f) => f.source === "radarr");
      setSelectedMoviesPaths(radarrFolders.map((folder) => folder.path));
      setCustomMoviesPaths([]);

      const sonarrFolders = preview.rootFolders.filter((f) => f.source === "sonarr");
      setSelectedSeriesPaths(sonarrFolders.map((folder) => folder.path));
      setSelectedAnimePaths([]);
      setCustomSeriesPaths([]);
      setCustomAnimePaths([]);

      goToStep(2);
    } catch (err) {
      setImportConnectError(err instanceof Error ? err.message : "Connection failed");
    } finally {
      setImportConnecting(false);
    }
  }, [client, sonarrUrl, sonarrApiKey, radarrUrl, radarrApiKey, goToStep]);

  // ── Import: Execute ─────────────────────────────────────────────────
  const buildSelectedImportPaths = useCallback(
    (selectedImportedPaths: string[], customPaths: string[]) => [
      ...selectedImportedPaths,
      ...customPaths.filter((path) => !selectedImportedPaths.includes(path)),
    ],
    [],
  );

  const finalSelectedMoviesPaths = buildSelectedImportPaths(
    selectedMoviesPaths,
    customMoviesPaths,
  );
  const finalSelectedSeriesPaths = buildSelectedImportPaths(
    selectedSeriesPaths,
    customSeriesPaths,
  );
  const finalSelectedAnimePaths = buildSelectedImportPaths(
    selectedAnimePaths,
    customAnimePaths,
  );

  const importedSonarrPaths = useMemo(
    () =>
      importPreview?.rootFolders
        .filter((folder) => folder.source === "sonarr")
        .map((folder) => folder.path) ?? [],
    [importPreview],
  );

  const handleImportExecute = useCallback(async () => {
    setImportExecuting(true);
    setImportExecuteError(null);
    try {
      const sonarr =
        sonarrUrl.trim() && sonarrApiKey.trim()
          ? { baseUrl: sonarrUrl.trim(), apiKey: sonarrApiKey.trim() }
          : undefined;
      const radarr =
        radarrUrl.trim() && radarrApiKey.trim()
          ? { baseUrl: radarrUrl.trim(), apiKey: radarrApiKey.trim() }
          : undefined;

      const { data, error } = await client
        .mutation(executeExternalImportMutation, {
          input: {
            sonarr: sonarr ?? null,
            radarr: radarr ?? null,
            selectedMoviesPaths: finalSelectedMoviesPaths,
            selectedSeriesPaths: finalSelectedSeriesPaths,
            selectedAnimePaths: finalSelectedAnimePaths,
            selectedDownloadClientDedupKeys: [...selectedDcKeys],
            selectedIndexerDedupKeys: [...selectedIdxKeys],
            downloadClientApiKeyOverrides: [...dcApiKeyOverrides.entries()].map(
              ([dedupKey, apiKey]) => ({ dedupKey, apiKey }),
            ),
          },
        })
        .toPromise();
      if (error) throw error;

      const result: ExternalImportResult = data.executeExternalImport;
      setImportResult(result);

      // Keep the wizard summary aligned with the default imported roots.
      if (finalSelectedMoviesPaths.length > 0) setMoviesPath(finalSelectedMoviesPaths[0]);
      if (finalSelectedSeriesPaths.length > 0) setSeriesPath(finalSelectedSeriesPaths[0]);
      if (finalSelectedAnimePaths.length > 0) setAnimePath(finalSelectedAnimePaths[0]);

      if (result.errors.length > 0) {
        setImportExecuteError(result.errors.join("; "));
      }

      goToStep(3); // → persona
    } catch (err) {
      setImportExecuteError(err instanceof Error ? err.message : "Import failed");
    } finally {
      setImportExecuting(false);
    }
  }, [
    client,
    sonarrUrl,
    sonarrApiKey,
    radarrUrl,
    radarrApiKey,
    finalSelectedMoviesPaths,
    finalSelectedSeriesPaths,
    finalSelectedAnimePaths,
    selectedDcKeys,
    selectedIdxKeys,
    dcApiKeyOverrides,
    goToStep,
  ]);

  // ── Complete setup ──────────────────────────────────────────────────
  const navigateAfterSetup = useCallback(() => {
    navigate(isReentry ? "/settings" : "/movies", { replace: true });
  }, [isReentry, navigate]);

  const finishSetup = useCallback(async (action: "finish" | "importOnly" = "finish") => {
    setFinishingAction(action);
    try {
      await client.mutation(completeSetupMutation, {}).toPromise();
    } catch {
      // Setup completion falling through to navigation keeps the wizard resilient.
    }
    navigateAfterSetup();
  }, [client, navigateAfterSetup]);

  const finishImportAndScan = useCallback(async () => {
    setFinishingAction("importAndScan");

    const selectedFacets = [
      finalSelectedMoviesPaths.length > 0
        ? { facet: "movie", label: t("setup.facetMovies") }
        : null,
      finalSelectedSeriesPaths.length > 0
        ? { facet: "series", label: t("setup.facetSeries") }
        : null,
      finalSelectedAnimePaths.length > 0
        ? { facet: "anime", label: t("setup.facetAnime") }
        : null,
    ].filter((value): value is { facet: "movie" | "series" | "anime"; label: string } => value !== null);

    try {
      await client.mutation(completeSetupMutation, {}).toPromise();

      await Promise.all(
        selectedFacets.map(async ({ facet, label }) => {
          try {
            const result = await client
              .mutation(scanLibraryMutation, { facet })
              .toPromise();
            if (result.error) throw result.error;
          } catch (error) {
            const message =
              error instanceof Error ? error.message : String(error ?? "");
            if (/library scan already running/i.test(message)) {
              toast.info(
                t("settings.libraryScanAlreadyRunning").replace("{{facet}}", label),
              );
              return;
            }

            toast.warning(
              message || t("settings.libraryScanFailed"),
            );
          }
        }),
      );
    } catch {
      // Still navigate into the app even if setup completion reporting fails.
    }

    navigateAfterSetup();
  }, [
    client,
    finalSelectedAnimePaths.length,
    finalSelectedMoviesPaths.length,
    finalSelectedSeriesPaths.length,
    navigateAfterSetup,
    t,
  ]);

  // ── Toggle helpers for import review ────────────────────────────────
  const toggleImportedPathSelection = useCallback(
    (
      setter: Dispatch<SetStateAction<string[]>>,
      path: string,
      importedPaths: string[],
    ) => {
      setter((prev) =>
        prev.includes(path)
          ? prev.filter((entry) => entry !== path)
          : importedPaths.filter((entry) => prev.includes(entry) || entry === path),
      );
    },
    [],
  );

  const toggleMoviesPath = useCallback(
    (path: string) => {
      const importedPaths = importPreview?.rootFolders
        .filter((folder) => folder.source === "radarr")
        .map((folder) => folder.path) ?? [];
      toggleImportedPathSelection(setSelectedMoviesPaths, path, importedPaths);
    },
    [importPreview, toggleImportedPathSelection],
  );

  const toggleSeriesPath = useCallback(
    (path: string) => {
      toggleImportedPathSelection(setSelectedSeriesPaths, path, importedSonarrPaths);
    },
    [importedSonarrPaths, toggleImportedPathSelection],
  );

  const toggleAnimePath = useCallback(
    (path: string) => {
      toggleImportedPathSelection(setSelectedAnimePaths, path, importedSonarrPaths);
    },
    [importedSonarrPaths, toggleImportedPathSelection],
  );

  const addCustomFacetPath = useCallback(
    (
      path: string,
      importedPaths: string[],
      customPaths: string[],
      setCustomPaths: Dispatch<SetStateAction<string[]>>,
      setSelectedImportedPaths: Dispatch<SetStateAction<string[]>>,
    ) => {
      const trimmed = path.trim();
      if (!trimmed) {
        return;
      }
      if (importedPaths.includes(trimmed)) {
        setSelectedImportedPaths((prev) =>
          prev.includes(trimmed)
            ? prev
            : importedPaths.filter((entry) => prev.includes(entry) || entry === trimmed),
        );
        return;
      }
      if (customPaths.includes(trimmed)) {
        return;
      }
      setCustomPaths((prev) => [...prev, trimmed]);
    },
    [],
  );

  const removeCustomFacetPath = useCallback(
    (path: string, setCustomPaths: Dispatch<SetStateAction<string[]>>) => {
      setCustomPaths((prev) => prev.filter((entry) => entry !== path));
    },
    [],
  );

  const addCustomMoviesPath = useCallback(
    (path: string) => {
      const importedPaths = importPreview?.rootFolders
        .filter((folder) => folder.source === "radarr")
        .map((folder) => folder.path) ?? [];
      addCustomFacetPath(
        path,
        importedPaths,
        customMoviesPaths,
        setCustomMoviesPaths,
        setSelectedMoviesPaths,
      );
    },
    [addCustomFacetPath, customMoviesPaths, importPreview],
  );

  const addCustomSeriesPath = useCallback(
    (path: string) => {
      addCustomFacetPath(
        path,
        importedSonarrPaths,
        customSeriesPaths,
        setCustomSeriesPaths,
        setSelectedSeriesPaths,
      );
    },
    [addCustomFacetPath, customSeriesPaths, importedSonarrPaths],
  );

  const addCustomAnimePath = useCallback(
    (path: string) => {
      addCustomFacetPath(
        path,
        importedSonarrPaths,
        customAnimePaths,
        setCustomAnimePaths,
        setSelectedAnimePaths,
      );
    },
    [addCustomFacetPath, customAnimePaths, importedSonarrPaths],
  );

  const removeCustomMoviesPath = useCallback(
    (path: string) => removeCustomFacetPath(path, setCustomMoviesPaths),
    [removeCustomFacetPath],
  );

  const removeCustomSeriesPath = useCallback(
    (path: string) => removeCustomFacetPath(path, setCustomSeriesPaths),
    [removeCustomFacetPath],
  );

  const removeCustomAnimePath = useCallback(
    (path: string) => removeCustomFacetPath(path, setCustomAnimePaths),
    [removeCustomFacetPath],
  );

  const toggleDcKey = useCallback((key: string) => {
    setSelectedDcKeys((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }, []);

  const toggleIdxKey = useCallback((key: string) => {
    setSelectedIdxKeys((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }, []);

  const setDcApiKey = useCallback((dedupKey: string, apiKey: string) => {
    setDcApiKeyOverrides((prev) => {
      const next = new Map(prev);
      if (apiKey) next.set(dedupKey, apiKey);
      else next.delete(dedupKey);
      return next;
    });
  }, []);

  // ── Render ──────────────────────────────────────────────────────────

  // Step mapping for progress bar (step 0 = welcome, not shown in bar)
  const progressStep = currentStep > 0 ? currentStep - 1 : -1;

  return (
    <div className="mx-auto flex min-h-screen w-full max-w-2xl flex-col items-center justify-center px-4 py-10">
      {currentStep > 0 && (
        <div className="mb-8 w-full">
          <SetupProgressBar currentStep={progressStep} stepLabels={stepLabels} />
        </div>
      )}

      {/* ── Step 0: Welcome (shared) ─────────────────────────────────── */}
      {currentStep === 0 && (
        <SetupWelcomeView
          t={t}
          onFreshSetup={() => goToStep(1, "fresh")}
          onImportSetup={() => goToStep(1, "import")}
          onSkip={finishSetup}
          skipping={finishing}
        />
      )}

      {/* ════════════════════════════════════════════════════════════════ */}
      {/* FRESH PATH                                                      */}
      {/* ════════════════════════════════════════════════════════════════ */}

      {currentStep === 1 && wizardPath === "fresh" && (
        <SetupPersonaView
          t={t}
          facetPrefs={facetPrefs}
          onFacetPrefsChange={(facet, prefs) =>
            setFacetPrefs((prev) => ({ ...prev, [facet]: prefs }))
          }
          onNext={() => saveFacetQualityPrefs(2)}
          onBack={() => goToStep(0)}
          onSkip={() => goToStep(2)}
          saving={personaSaving}
        />
      )}

      {currentStep === 2 && wizardPath === "fresh" && (
        <SetupMediaPathsView
          t={t}
          moviesPath={moviesPath}
          seriesPath={seriesPath}
          animePath={animePath}
          onMoviesPathChange={setMoviesPath}
          onSeriesPathChange={setSeriesPath}
          onAnimePathChange={setAnimePath}
          onNext={saveMediaPaths}
          onBack={() => goToStep(1)}
          onSkip={() => goToStep(3)}
          saving={mediaPathsSaving}
          error={mediaPathsError}
        />
      )}

      {currentStep === 3 && wizardPath === "fresh" && (
        <SetupPluginsView
          t={t}
          plugins={plugins}
          loading={pluginsLoading}
          refreshing={pluginsRefreshing}
          mutatingPluginIds={mutatingPluginIds}
          pluginProgress={pluginProgress}
          pluginErrors={pluginErrors}
          error={pluginsError}
          onRefreshRegistry={refreshPluginsRegistry}
          onInstallPlugin={installPlugin}
          onUninstallPlugin={uninstallPlugin}
          onNext={() => goToStep(4)}
          onBack={() => goToStep(2)}
        />
      )}

      {currentStep === 4 && wizardPath === "fresh" && (
        <SetupDownloadClientView
          t={t}
          draft={dcDraft}
          downloadClientTypeOptions={availableDcTypeOptions}
          onDraftChange={(updates) =>
            setDcDraft((prev) => {
              const next = { ...prev, ...updates };
              if (updates.clientType && updates.clientType !== prev.clientType) {
                const prevDefault = DEFAULT_PORT_FOR_CLIENT_TYPE[prev.clientType] ?? "8080";
                if (prev.port === "" || prev.port === prevDefault) {
                  next.port = DEFAULT_PORT_FOR_CLIENT_TYPE[updates.clientType] ?? "8080";
                }
              }
              return next;
            })
          }
          onTestConnection={dcSaved ? testDownloadClient : handleDcTestAndSave}
          onNext={() => goToStep(5)}
          onBack={() => goToStep(3)}
          onSkip={() => goToStep(5)}
          testing={dcTesting}
          testResult={dcTestResult}
          saving={dcSaving}
          saved={dcSaved}
          error={dcError}
        />
      )}

      {currentStep === 5 && wizardPath === "fresh" && (
        <SetupIndexerView
          t={t}
          name={idxName}
          providerType={idxProviderType}
          configValues={idxConfigValues}
          providerOptions={idxProviderOptions}
          onNameChange={handleIdxNameChange}
          onProviderTypeChange={handleIdxProviderTypeChange}
          onConfigValueChange={handleIdxConfigValueChange}
          onTestConnection={idxSaved ? testIndexer : handleIdxTestAndSave}
          onNext={() => goToStep(6)}
          onBack={() => goToStep(4)}
          onSkip={() => goToStep(6)}
          testing={idxTesting}
          testResult={idxTestResult}
          saving={idxSaving}
          saved={idxSaved}
          error={idxError}
        />
      )}

      {currentStep === 6 && wizardPath === "fresh" && (
        <SetupSummaryView
          t={t}
          facetPrefs={facetPrefs}
          moviesPaths={[moviesPath]}
          seriesPaths={[seriesPath]}
          animePaths={animePath ? [animePath] : []}
          downloadClientName={dcDraft.name || dcDraft.clientType}
          indexerName={idxName || idxProviderType}
          onFinish={finishSetup}
          onBack={() => goToStep(5)}
          finishing={finishing}
          finishingAction={finishingAction}
        />
      )}

      {/* ════════════════════════════════════════════════════════════════ */}
      {/* IMPORT PATH                                                     */}
      {/* ════════════════════════════════════════════════════════════════ */}

      {currentStep === 1 && wizardPath === "import" && (
        <SetupImportConnectView
          t={t}
          sonarrUrl={sonarrUrl}
          sonarrApiKey={sonarrApiKey}
          radarrUrl={radarrUrl}
          radarrApiKey={radarrApiKey}
          onSonarrUrlChange={setSonarrUrl}
          onSonarrApiKeyChange={setSonarrApiKey}
          onRadarrUrlChange={setRadarrUrl}
          onRadarrApiKeyChange={setRadarrApiKey}
          onConnect={handleImportConnect}
          onBack={() => goToStep(0)}
          connecting={importConnecting}
          error={importConnectError}
        />
      )}

      {currentStep === 2 && wizardPath === "import" && importPreview && (
        <SetupImportReviewView
          t={t}
          preview={importPreview}
          selectedMoviesPaths={selectedMoviesPaths}
          selectedSeriesPaths={selectedSeriesPaths}
          selectedAnimePaths={selectedAnimePaths}
          customMoviesPaths={customMoviesPaths}
          customSeriesPaths={customSeriesPaths}
          customAnimePaths={customAnimePaths}
          selectedDcKeys={selectedDcKeys}
          selectedIdxKeys={selectedIdxKeys}
          dcApiKeyOverrides={dcApiKeyOverrides}
          onToggleMoviesPath={toggleMoviesPath}
          onToggleSeriesPath={toggleSeriesPath}
          onToggleAnimePath={toggleAnimePath}
          onAddCustomMoviesPath={addCustomMoviesPath}
          onAddCustomSeriesPath={addCustomSeriesPath}
          onAddCustomAnimePath={addCustomAnimePath}
          onRemoveCustomMoviesPath={removeCustomMoviesPath}
          onRemoveCustomSeriesPath={removeCustomSeriesPath}
          onRemoveCustomAnimePath={removeCustomAnimePath}
          onToggleDc={toggleDcKey}
          onToggleIdx={toggleIdxKey}
          onSetDcApiKey={setDcApiKey}
          onImport={handleImportExecute}
          onBack={() => goToStep(1)}
          importing={importExecuting}
          error={importExecuteError}
        />
      )}

      {currentStep === 3 && wizardPath === "import" && (
        <SetupPersonaView
          t={t}
          facetPrefs={facetPrefs}
          onFacetPrefsChange={(facet, prefs) =>
            setFacetPrefs((prev) => ({ ...prev, [facet]: prefs }))
          }
          onNext={() => saveFacetQualityPrefs(4)}
          onBack={() => goToStep(2)}
          saving={personaSaving}
        />
      )}

      {currentStep === 4 && wizardPath === "import" && (
        <SetupSummaryView
          t={t}
          facetPrefs={facetPrefs}
          moviesPaths={selectedMoviesPaths}
          seriesPaths={selectedSeriesPaths}
          animePaths={selectedAnimePaths}
          downloadClientName=""
          indexerName=""
          importedDcCount={importResult?.downloadClientsCreated}
          importedIdxCount={importResult?.indexersCreated}
          onImportOnly={() => finishSetup("importOnly")}
          onImportAndScan={finishImportAndScan}
          onBack={() => goToStep(3)}
          finishing={finishing}
          finishingAction={finishingAction}
        />
      )}
    </div>
  );
}
