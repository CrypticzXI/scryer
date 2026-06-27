import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { Dispatch, SetStateAction } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";
import { toast } from "sonner";
import { useClient } from "urql";

import {
  externalImportMonitorWarmupStatusQuery,
  externalImportMonitorWarmupProgressSubscription,
  qualityProfilesInitQuery,
  setupStatusQuery,
  setupWizardProviderTypesInitQuery,
} from "@/lib/graphql/queries";
import {
  saveQualityProfileSettingsMutation,
  updateLibraryPathsMutation,
  completeSetupMutation,
  previewExternalImportMutation,
  executeExternalImportMutation,
  startExternalImportMonitorWarmupMutation,
  cancelExternalImportMonitorWarmupMutation,
  finalizeExternalImportMutation,
  scanLibraryMutation,
} from "@/lib/graphql/mutations";
import { wsClient } from "@/lib/graphql/ws-client";
import { buildDownloadClientTypeOptions } from "@/lib/utils/download-clients";
import { useDownloadClientSetup } from "@/lib/hooks/use-download-client-setup";
import {
  useIndexerSetup,
  type SetupIndexerProviderOption,
} from "@/lib/hooks/use-indexer-setup";
import { usePluginManagement } from "@/lib/hooks/use-plugin-management";
import { localPathStyleFromRuntimeValue } from "@/lib/utils/local-path-style";
import {
  qualityProfileSettingsToEntries,
  qualityProfileEntryToMutationInput,
} from "@/lib/utils/quality-profiles";
import type {
  FacetQualityPrefs,
  QualityTargetId,
  ViewCategoryId,
} from "@/lib/types/quality-profiles";
import type {
  ExternalImportConnection,
  ExternalImportMonitorWarmupProgress,
  ExternalImportPreview,
  ExternalImportResult,
} from "@/lib/types/external-import";
import type { ProviderTypeInfo } from "@/lib/types";

import ScryerLogo from "@/components/scryer-logo";
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
import { SetupRestoreView } from "./setup-restore-view";
import { findMissingExternalImportApiKeyRequirement } from "./setup-import-api-key-requirements";

const FALLBACK_PROVIDER_OPTIONS: SetupIndexerProviderOption[] = [];

function defaultLibraryIdForFacet(facet: "movie" | "series" | "anime") {
  return `${facet}_default_library`;
}

type ExternalImportMonitorWarmupProgressSubscriptionResult = {
  data?: {
    externalImportMonitorWarmupProgress?: ExternalImportMonitorWarmupProgress;
  };
};

interface SetupWizardContainerProps {
  t: (
    key: string,
    values?: Record<string, string | number | boolean | null | undefined>,
  ) => string;
  isReentry?: boolean;
  onBackendRestarting: () => void;
}

function formatQualityTarget(target: QualityTargetId): string {
  switch (target) {
    case "8k":
      return "8K";
    case "4k":
      return "4K";
    case "1080p":
      return "1080P";
  }
  return target;
}

export function SetupWizardContainer({
  t,
  isReentry,
  onBackendRestarting,
}: SetupWizardContainerProps) {
  const client = useClient();
  const navigate = useNavigate();

  // ── Wizard path + step (URL-driven for browser back/forward) ──────
  const [searchParams, setSearchParams] = useSearchParams();
  const wizardPath: "fresh" | "import" | "restore" =
    searchParams.get("path") === "import"
      ? "import"
      : searchParams.get("path") === "restore"
        ? "restore"
        : "fresh";
  const currentStep = parseInt(searchParams.get("step") || "0", 10);
  const [canRestoreSetup, setCanRestoreSetup] = useState(false);
  const [restoreAvailabilityChecked, setRestoreAvailabilityChecked] =
    useState(false);

  const goToStep = useCallback(
    (step: number, path?: "fresh" | "import" | "restore") => {
      const p = path ?? wizardPath;
      if (step === 0) {
        setSearchParams({});
      } else {
        setSearchParams({ path: p, step: String(step) });
      }
    },
    [wizardPath, setSearchParams],
  );

  useEffect(() => {
    let cancelled = false;
    setCanRestoreSetup(false);
    setRestoreAvailabilityChecked(false);

    client
      .query(setupStatusQuery, {}, { requestPolicy: "network-only" })
      .toPromise()
      .then(({ data }) => {
        if (cancelled) return;
        setCanRestoreSetup(data?.setupStatus?.setupComplete === false);
      })
      .catch(() => {
        if (cancelled) return;
        setCanRestoreSetup(false);
      })
      .finally(() => {
        if (cancelled) return;
        setRestoreAvailabilityChecked(true);
      });

    return () => {
      cancelled = true;
    };
  }, [client]);

  useEffect(() => {
    if (wizardPath !== "restore" || !restoreAvailabilityChecked || canRestoreSetup) {
      return;
    }

    const nextSearchParams = new URLSearchParams();
    if (isReentry) {
      nextSearchParams.set("reentry", "1");
    }
    setSearchParams(nextSearchParams, { replace: true });
  }, [
    canRestoreSetup,
    isReentry,
    restoreAvailabilityChecked,
    setSearchParams,
    wizardPath,
  ]);

  // ── Step 1 (fresh) / Step 3 (import): Quality Preferences ─────────
  const [facetPrefs, setFacetPrefs] = useState<
    Record<ViewCategoryId, FacetQualityPrefs>
  >({
    movie: { quality: "4k", persona: "balanced" },
    series: { quality: "4k", persona: "balanced" },
    anime: { quality: "1080p", persona: "balanced" },
  });
  const [personaSaving, setPersonaSaving] = useState(false);

  // ── Step 2 (fresh): Media Paths ─────────────────────────────────────
  const [moviesPath, setMoviesPath] = useState("/data/movies");
  const [seriesPath, setSeriesPath] = useState("/data/series");
  const [animePath, setAnimePath] = useState("");
  const [mediaPathsSaving, setMediaPathsSaving] = useState(false);
  const [mediaPathsError, setMediaPathsError] = useState<string | null>(null);

  // ── Step 4 (fresh): Download Client ─────────────────────────────────
  const {
    dcDraft,
    dcLocalPathStyle,
    setDcLocalPathStyle,
    setDcTypeOptions,
    availableDcTypeOptions,
    dcTesting,
    dcTestResult,
    dcSaving,
    dcSaved,
    dcError,
    handleDcDraftChange,
    testDownloadClient,
    handleDcTestAndSave,
  } = useDownloadClientSetup({ client });

  // ── Step 5 (fresh): Indexer ─────────────────────────────────────────
  const {
    idxName,
    idxProviderType,
    idxConfigValues,
    idxProviderOptions,
    setIdxProviderOptions,
    idxTesting,
    idxTestResult,
    idxSaving,
    idxSaved,
    idxError,
    indexerProviderConfigFieldsByType,
    handleIdxNameChange,
    handleIdxProviderTypeChange,
    handleIdxConfigValueChange,
    testIndexer,
    handleIdxTestAndSave,
  } = useIndexerSetup({ client, t });

  // ── Import: Connect ─────────────────────────────────────────────────
  const [sonarrUrl, setSonarrUrl] = useState("");
  const [sonarrApiKey, setSonarrApiKey] = useState("");
  const [radarrUrl, setRadarrUrl] = useState("");
  const [radarrApiKey, setRadarrApiKey] = useState("");
  const [prowlarrUrl, setProwlarrUrl] = useState("");
  const [prowlarrApiKey, setProwlarrApiKey] = useState("");
  const [importConnecting, setImportConnecting] = useState(false);
  const [importConnectError, setImportConnectError] = useState<string | null>(
    null,
  );
  const [importConnectServiceErrors, setImportConnectServiceErrors] = useState<{
    sonarr: string | null;
    radarr: string | null;
    prowlarr: string | null;
  }>({
    sonarr: null,
    radarr: null,
    prowlarr: null,
  });

  // ── Import: Preview / Review ────────────────────────────────────────
  const [importPreview, setImportPreview] =
    useState<ExternalImportPreview | null>(null);
  const [selectedMoviesPaths, setSelectedMoviesPaths] = useState<string[]>([]);
  const [selectedSeriesPaths, setSelectedSeriesPaths] = useState<string[]>([]);
  const [customMoviesPaths, setCustomMoviesPaths] = useState<string[]>([]);
  const [customSeriesPaths, setCustomSeriesPaths] = useState<string[]>([]);
  const [selectedDcKeys, setSelectedDcKeys] = useState<Set<string>>(new Set());
  const [selectedIdxKeys, setSelectedIdxKeys] = useState<Set<string>>(
    new Set(),
  );
  // User-supplied API keys for clients whose keys were masked by Sonarr/Radarr.
  const [dcApiKeyOverrides, setDcApiKeyOverrides] = useState<
    Map<string, string>
  >(new Map());
  const [dcPasswordOverrides, setDcPasswordOverrides] = useState<
    Map<string, string>
  >(new Map());
  const [idxApiKeyOverrides, setIdxApiKeyOverrides] = useState<
    Map<string, string>
  >(new Map());
  const [selectedAnimePaths, setSelectedAnimePaths] = useState<string[]>([]);
  const [customAnimePaths, setCustomAnimePaths] = useState<string[]>([]);
  const [importExecuting, setImportExecuting] = useState(false);
  const [importExecuteError, setImportExecuteError] = useState<string | null>(
    null,
  );
  const [importResult, setImportResult] = useState<ExternalImportResult | null>(
    null,
  );
  const [importWarmupProgress, setImportWarmupProgress] =
    useState<ExternalImportMonitorWarmupProgress | null>(null);
  const [importWarmupError, setImportWarmupError] = useState<string | null>(
    null,
  );
  const warmupSubscriptionRef = useRef<(() => void) | null>(null);

  // ── Summary / Finish ────────────────────────────────────────────────
  const [finishingAction, setFinishingAction] = useState<
    "finish" | "importOnly" | "importAndScan" | null
  >(null);
  const finishing = finishingAction !== null;

  const stopImportWarmupProgressSubscription = useCallback(() => {
    if (warmupSubscriptionRef.current) {
      warmupSubscriptionRef.current();
      warmupSubscriptionRef.current = null;
    }
  }, []);

  const externalImportConnections = useMemo<{
    sonarr: ExternalImportConnection | null;
    radarr: ExternalImportConnection | null;
  }>(
    () => ({
      sonarr:
        sonarrUrl.trim() && sonarrApiKey.trim()
          ? { baseUrl: sonarrUrl.trim(), apiKey: sonarrApiKey.trim() }
          : null,
      radarr:
        radarrUrl.trim() && radarrApiKey.trim()
          ? { baseUrl: radarrUrl.trim(), apiKey: radarrApiKey.trim() }
          : null,
    }),
    [radarrApiKey, radarrUrl, sonarrApiKey, sonarrUrl],
  );

  const beginImportWarmupProgressSubscription = useCallback(
    (
      sessionId: string,
      initialSnapshot?: ExternalImportMonitorWarmupProgress | null,
    ) => {
      stopImportWarmupProgressSubscription();
      if (initialSnapshot) {
        setImportWarmupProgress(initialSnapshot);
      }
      const unsubscribe = wsClient.subscribe(
        {
          query: externalImportMonitorWarmupProgressSubscription,
          variables: { sessionId },
        },
        {
          next: (
            result: ExternalImportMonitorWarmupProgressSubscriptionResult,
          ) => {
            const snapshot = result.data?.externalImportMonitorWarmupProgress;
            if (!snapshot) {
              return;
            }

            setImportWarmupProgress(snapshot);
            setImportWarmupError(snapshot.errorMessage ?? null);
            if (
              snapshot.status === "completed" ||
              snapshot.status === "failed" ||
              snapshot.status === "canceled"
            ) {
              stopImportWarmupProgressSubscription();
            }
          },
          error: (error) => {
            stopImportWarmupProgressSubscription();
            setImportWarmupError(
              error instanceof Error ? error.message : t("setup.connectError"),
            );
          },
          complete: () => {
            warmupSubscriptionRef.current = null;
          },
        },
      );
      warmupSubscriptionRef.current = unsubscribe;
    },
    [stopImportWarmupProgressSubscription, t],
  );

  const refreshImportWarmupStatus = useCallback(
    async (sessionId: string) => {
      const { data, error } = await client
        .query(
          externalImportMonitorWarmupStatusQuery,
          { sessionId },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) {
        throw error;
      }

      const snapshot = data?.externalImportMonitorWarmupStatus as
        | ExternalImportMonitorWarmupProgress
        | undefined;
      if (!snapshot) {
        return null;
      }

      setImportWarmupProgress(snapshot);
      setImportWarmupError(snapshot.errorMessage ?? null);
      return snapshot;
    },
    [client],
  );

  const refreshProviderOptions = useCallback(async () => {
    try {
      const { data, error } = await client
        .query(setupWizardProviderTypesInitQuery, {})
        .toPromise();
      if (
        error &&
        !data?.downloadClientProviderTypes &&
        !data?.indexerProviderTypes
      )
        throw error;

      setDcLocalPathStyle(
        localPathStyleFromRuntimeValue(data?.runtimeInfo?.runtimePathStyle),
      );
      setDcTypeOptions(
        buildDownloadClientTypeOptions(
          (data?.downloadClientProviderTypes as
            | ProviderTypeInfo[]
            | undefined) ?? [],
        ),
      );

      if (data?.indexerProviderTypes?.length) {
        setIdxProviderOptions(
          data.indexerProviderTypes.map((provider: ProviderTypeInfo) => ({
            value: provider.providerType,
            label: provider.name,
            defaultBaseUrl: provider.defaultBaseUrl || undefined,
            configFields: provider.configFields ?? [],
          })),
        );
      } else {
        setIdxProviderOptions(FALLBACK_PROVIDER_OPTIONS);
      }
    } catch {
      setDcTypeOptions(buildDownloadClientTypeOptions([]));
      setIdxProviderOptions(FALLBACK_PROVIDER_OPTIONS);
    }
  }, [client]);

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
  } = usePluginManagement({ client, t, refreshProviderOptions });

  // ── Step labels per path ────────────────────────────────────────────
  const stepLabels =
    wizardPath === "import"
      ? [
          t("setup.stepConnect"),
          t("setup.stepReview"),
          t("setup.stepPersona"),
          t("setup.stepSummary"),
        ]
      : wizardPath === "restore"
        ? [t("setup.stepRestore")]
        : [
            t("setup.stepPersona"),
            t("setup.stepMediaPaths"),
            t("setup.stepPlugins"),
            t("setup.stepDownloadClient"),
            t("setup.stepIndexer"),
            t("setup.stepSummary"),
          ];

  // ── Quality preferences save (per-facet) ────────────────────────────
  const saveFacetQualityPrefs = useCallback(
    async (nextStep: number) => {
      setPersonaSaving(true);
      try {
        const { data } = await client
          .query(qualityProfilesInitQuery, {})
          .toPromise();
        const existingProfiles = qualityProfileSettingsToEntries(
          data?.qualityProfileSettings,
        );

        // Build per-facet profiles from templates
        const WIZARD_FACETS: { facet: ViewCategoryId; name: string }[] = [
          { facet: "movie", name: "Movies" },
          { facet: "series", name: "Series" },
          { facet: "anime", name: "Anime" },
        ];
        const wizardProfileIds = WIZARD_FACETS.map((f) => `wizard-${f.facet}`);
        const builtinProfileIds = ["8k", "4k", "1080p"];
        const keptProfiles = existingProfiles.filter(
          (p) =>
            !wizardProfileIds.includes(p.id) &&
            !builtinProfileIds.includes(p.id),
        );

        for (const { facet, name } of WIZARD_FACETS) {
          const prefs = facetPrefs[facet];
          const template = existingProfiles.find((p) => p.id === prefs.quality);
          if (template) {
            const profileName = `${name} (${formatQualityTarget(prefs.quality)})`;
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

  // ── Import: Connect & Scan ──────────────────────────────────────────
  const handleImportConnect = useCallback(async () => {
    setImportConnecting(true);
    setImportConnectError(null);
    setImportConnectServiceErrors({
      sonarr: null,
      radarr: null,
      prowlarr: null,
    });
    try {
      const sonarr =
        sonarrUrl.trim() && sonarrApiKey.trim()
          ? { baseUrl: sonarrUrl.trim(), apiKey: sonarrApiKey.trim() }
          : undefined;
      const radarr =
        radarrUrl.trim() && radarrApiKey.trim()
          ? { baseUrl: radarrUrl.trim(), apiKey: radarrApiKey.trim() }
          : undefined;
      const prowlarr =
        prowlarrUrl.trim() && prowlarrApiKey.trim()
          ? { baseUrl: prowlarrUrl.trim(), apiKey: prowlarrApiKey.trim() }
          : undefined;

      const { data, error } = await client
        .mutation(previewExternalImportMutation, {
          input: {
            sonarr: sonarr ?? null,
            radarr: radarr ?? null,
            prowlarr: prowlarr ?? null,
          },
        })
        .toPromise();
      if (error) throw error;

      const preview: ExternalImportPreview = data.previewExternalImport;

      const normalizeConnectError = (
        providerLabel: string,
        raw: string | null | undefined,
        fallback: string,
      ) => {
        const message = raw?.trim() || fallback;
        const stripped = message.replace(/^(repository|validation):\s*/i, "");
        if (/^invalid api key$/i.test(stripped)) {
          return `${providerLabel} API key is invalid.`;
        }
        return stripped;
      };

      const sonarrError =
        !preview.sonarrConnected && sonarr
          ? normalizeConnectError(
              "Sonarr",
              preview.sonarrError,
              "Could not connect to Sonarr. Check the URL and API key.",
            )
          : null;
      const radarrError =
        !preview.radarrConnected && radarr
          ? normalizeConnectError(
              "Radarr",
              preview.radarrError,
              "Could not connect to Radarr. Check the URL and API key.",
            )
          : null;
      const prowlarrError =
        !preview.prowlarrConnected && prowlarr
          ? normalizeConnectError(
              "Prowlarr",
              preview.prowlarrError,
              "Could not connect to Prowlarr. Check the URL and API key.",
            )
          : null;

      if (sonarrError || radarrError || prowlarrError) {
        setImportConnectServiceErrors({
          sonarr: sonarrError,
          radarr: radarrError,
          prowlarr: prowlarrError,
        });
        const failedProviders = [
          sonarrError ? "Sonarr" : null,
          radarrError ? "Radarr" : null,
          prowlarrError ? "Prowlarr" : null,
        ].filter((value): value is string => value !== null);
        setImportConnectError(
          failedProviders.length === 1
            ? `${failedProviders[0]} connection failed.`
            : `Some connections failed: ${failedProviders.join(", ")}.`,
        );
        setImportConnecting(false);
        return;
      }

      setImportPreview(preview);
      setDcApiKeyOverrides(new Map());
      setDcPasswordOverrides(new Map());
      setIdxApiKeyOverrides(new Map());

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
      const radarrFolders = preview.rootFolders.filter(
        (f) => f.source === "radarr",
      );
      setSelectedMoviesPaths(radarrFolders.map((folder) => folder.path));
      setCustomMoviesPaths([]);

      const sonarrFolders = preview.rootFolders.filter(
        (f) => f.source === "sonarr",
      );
      setSelectedSeriesPaths(sonarrFolders.map((folder) => folder.path));
      setSelectedAnimePaths([]);
      setCustomSeriesPaths([]);
      setCustomAnimePaths([]);

      goToStep(2);
    } catch (err) {
      setImportConnectError(
        err instanceof Error ? err.message : "Connection failed",
      );
    } finally {
      setImportConnecting(false);
    }
  }, [
    client,
    sonarrUrl,
    sonarrApiKey,
    radarrUrl,
    radarrApiKey,
    prowlarrUrl,
    prowlarrApiKey,
    goToStep,
  ]);

  useEffect(() => {
    if (wizardPath !== "import" || currentStep !== 2 || !importPreview) {
      return;
    }
    if (
      !externalImportConnections.sonarr &&
      !externalImportConnections.radarr
    ) {
      return;
    }
    if (
      importWarmupProgress?.status === "queued" ||
      importWarmupProgress?.status === "running" ||
      importWarmupProgress?.status === "completed" ||
      importWarmupProgress?.status === "failed" ||
      importWarmupProgress?.status === "canceled"
    ) {
      return;
    }

    let canceled = false;
    void (async () => {
      try {
        const { data, error } = await client
          .mutation(startExternalImportMonitorWarmupMutation, {
            input: {
              sonarr: externalImportConnections.sonarr,
              radarr: externalImportConnections.radarr,
            },
          })
          .toPromise();
        if (error) {
          throw error;
        }

        const snapshot = data?.startExternalImportMonitorWarmup as
          | ExternalImportMonitorWarmupProgress
          | undefined;
        if (!snapshot || canceled) {
          return;
        }

        setImportWarmupProgress(snapshot);
        setImportWarmupError(snapshot.errorMessage ?? null);
        if (snapshot.status === "queued" || snapshot.status === "running") {
          beginImportWarmupProgressSubscription(snapshot.sessionId, snapshot);
        } else {
          stopImportWarmupProgressSubscription();
        }
      } catch (error) {
        if (!canceled) {
          setImportWarmupError(
            error instanceof Error ? error.message : t("setup.connectError"),
          );
        }
      }
    })();

    return () => {
      canceled = true;
    };
  }, [
    beginImportWarmupProgressSubscription,
    client,
    currentStep,
    externalImportConnections.radarr,
    externalImportConnections.sonarr,
    importPreview,
    importWarmupProgress?.status,
    stopImportWarmupProgressSubscription,
    t,
    wizardPath,
  ]);

  useEffect(() => {
    if (wizardPath === "import" && currentStep >= 2) {
      return;
    }

    const sessionId = importWarmupProgress?.sessionId;
    if (!sessionId) {
      stopImportWarmupProgressSubscription();
      return;
    }

    stopImportWarmupProgressSubscription();
    if (
      importWarmupProgress.status !== "completed" &&
      importWarmupProgress.status !== "failed" &&
      importWarmupProgress.status !== "canceled"
    ) {
      void client
        .mutation(cancelExternalImportMonitorWarmupMutation, {
          sessionId,
        })
        .toPromise();
    }
    setImportWarmupProgress(null);
    setImportWarmupError(null);
  }, [
    client,
    currentStep,
    importWarmupProgress,
    stopImportWarmupProgressSubscription,
    wizardPath,
  ]);

  useEffect(() => {
    const sessionId = importWarmupProgress?.sessionId;
    if (!sessionId) {
      return;
    }
    if (
      importWarmupProgress.status === "completed" ||
      importWarmupProgress.status === "failed" ||
      importWarmupProgress.status === "canceled"
    ) {
      return;
    }

    let canceled = false;
    const sync = async () => {
      try {
        const snapshot = await refreshImportWarmupStatus(sessionId);
        if (!snapshot || canceled) {
          return;
        }
        if (
          snapshot.status === "completed" ||
          snapshot.status === "failed" ||
          snapshot.status === "canceled"
        ) {
          stopImportWarmupProgressSubscription();
        }
      } catch (error) {
        if (!canceled) {
          console.warn("[setup] failed to refresh import warmup status", error);
        }
      }
    };

    void sync();
    const intervalId = window.setInterval(() => {
      void sync();
    }, 3000);

    return () => {
      canceled = true;
      window.clearInterval(intervalId);
    };
  }, [
    importWarmupProgress?.sessionId,
    importWarmupProgress?.status,
    refreshImportWarmupStatus,
    stopImportWarmupProgressSubscription,
  ]);

  useEffect(
    () => () => {
      stopImportWarmupProgressSubscription();
    },
    [stopImportWarmupProgressSubscription],
  );

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
    const missingApiKeyRequirement = importPreview
      ? findMissingExternalImportApiKeyRequirement({
          preview: importPreview,
          selectedDcKeys,
          selectedIdxKeys,
          dcApiKeyOverrides,
          dcPasswordOverrides,
          idxApiKeyOverrides,
          indexerProviderConfigFieldsByType,
        })
      : null;
    if (missingApiKeyRequirement) {
      const missingCredentialMessage =
        missingApiKeyRequirement.kind === "password"
          ? t("status.passwordRequired")
          : missingApiKeyRequirement.isProwlarr
            ? t("setup.prowlarrApiKeyRequired", {
                name: missingApiKeyRequirement.name,
              })
            : t("setup.apiKeyMasked");
      setImportExecuteError(missingCredentialMessage);
      return;
    }

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
      const prowlarr =
        prowlarrUrl.trim() && prowlarrApiKey.trim()
          ? { baseUrl: prowlarrUrl.trim(), apiKey: prowlarrApiKey.trim() }
          : undefined;
      const downloadClientDedupKeys = new Set(
        importPreview?.downloadClients.map(
          (downloadClient) => downloadClient.dedupKey,
        ) ?? [],
      );
      const indexerDedupKeys = new Set(
        importPreview?.indexers.map((indexer) => indexer.dedupKey) ?? [],
      );

      const { data, error } = await client
        .mutation(executeExternalImportMutation, {
          input: {
            sonarr: sonarr ?? null,
            radarr: radarr ?? null,
            prowlarr: prowlarr ?? null,
            selectedMoviesPaths: finalSelectedMoviesPaths,
            selectedSeriesPaths: finalSelectedSeriesPaths,
            selectedAnimePaths: finalSelectedAnimePaths,
            selectedDownloadClientDedupKeys: [...selectedDcKeys],
            selectedIndexerDedupKeys: [...selectedIdxKeys],
            downloadClientApiKeyOverrides: [...dcApiKeyOverrides.entries()]
              .filter(([dedupKey]) => downloadClientDedupKeys.has(dedupKey))
              .map(([dedupKey, apiKey]) => ({ dedupKey, apiKey })),
            downloadClientPasswordOverrides: [...dcPasswordOverrides.entries()]
              .filter(([dedupKey]) => downloadClientDedupKeys.has(dedupKey))
              .map(([dedupKey, password]) => ({ dedupKey, password })),
            indexerApiKeyOverrides: [...idxApiKeyOverrides.entries()]
              .filter(([dedupKey]) => indexerDedupKeys.has(dedupKey))
              .map(([dedupKey, apiKey]) => ({ dedupKey, apiKey })),
          },
        })
        .toPromise();
      if (error) throw error;

      const result: ExternalImportResult = data.executeExternalImport;
      setImportResult(result);

      // Keep the wizard summary aligned with the default imported roots.
      if (finalSelectedMoviesPaths.length > 0)
        setMoviesPath(finalSelectedMoviesPaths[0]);
      if (finalSelectedSeriesPaths.length > 0)
        setSeriesPath(finalSelectedSeriesPaths[0]);
      if (finalSelectedAnimePaths.length > 0)
        setAnimePath(finalSelectedAnimePaths[0]);

      if (result.errors.length > 0) {
        setImportExecuteError(result.errors.join("; "));
      }

      goToStep(3); // → persona
    } catch (err) {
      setImportExecuteError(
        err instanceof Error ? err.message : "Import failed",
      );
    } finally {
      setImportExecuting(false);
    }
  }, [
    client,
    sonarrUrl,
    sonarrApiKey,
    radarrUrl,
    radarrApiKey,
    prowlarrUrl,
    prowlarrApiKey,
    finalSelectedMoviesPaths,
    finalSelectedSeriesPaths,
    finalSelectedAnimePaths,
    selectedDcKeys,
    selectedIdxKeys,
    dcApiKeyOverrides,
    dcPasswordOverrides,
    idxApiKeyOverrides,
    indexerProviderConfigFieldsByType,
    importPreview,
    t,
    goToStep,
  ]);

  // ── Complete setup ──────────────────────────────────────────────────
  const navigateAfterSetup = useCallback(() => {
    navigate(isReentry ? "/settings" : "/movies", { replace: true });
  }, [isReentry, navigate]);

  const finalizeImportedMonitorSnapshots = useCallback(async () => {
    const { data, error } = await client
      .mutation(finalizeExternalImportMutation, {
        input: {
          sonarr: externalImportConnections.sonarr,
          radarr: externalImportConnections.radarr,
          monitorWarmupSessionId: importWarmupProgress?.sessionId ?? null,
          selectedMoviesPaths: finalSelectedMoviesPaths,
          selectedSeriesPaths: finalSelectedSeriesPaths,
          selectedAnimePaths: finalSelectedAnimePaths,
        },
      })
      .toPromise();
    if (error) {
      throw error;
    }
    if (!data?.finalizeExternalImport?.finalized) {
      throw new Error(t("setup.importFinalizeFailed"));
    }
  }, [
    client,
    externalImportConnections.radarr,
    externalImportConnections.sonarr,
    finalSelectedAnimePaths,
    finalSelectedMoviesPaths,
    finalSelectedSeriesPaths,
    importWarmupProgress?.sessionId,
    t,
  ]);

  const finishSetup = useCallback(
    async (action: "finish" | "importOnly" = "finish") => {
      setFinishingAction(action);
      try {
        if (action === "importOnly") {
          await finalizeImportedMonitorSnapshots();
        }
        const { data, error } = await client
          .mutation(completeSetupMutation, {})
          .toPromise();
        if (error) {
          throw error;
        }
        if (!data?.completeSetup?.completed) {
          throw new Error(t("setup.connectError"));
        }
        navigateAfterSetup();
      } catch (error) {
        if (action === "importOnly") {
          toast.warning(
            error instanceof Error
              ? error.message
              : t("setup.importFinalizeFailed"),
          );
        } else {
          navigateAfterSetup();
        }
      } finally {
        setFinishingAction(null);
      }
    },
    [client, finalizeImportedMonitorSnapshots, navigateAfterSetup, t],
  );

  const finishImportAndScan = useCallback(async () => {
    setFinishingAction("importAndScan");

    const selectedFacets = [
      finalSelectedMoviesPaths.length > 0
        ? {
            facet: "movie",
            libraryId: defaultLibraryIdForFacet("movie"),
            label: t("setup.facetMovies"),
          }
        : null,
      finalSelectedSeriesPaths.length > 0
        ? {
            facet: "series",
            libraryId: defaultLibraryIdForFacet("series"),
            label: t("setup.facetSeries"),
          }
        : null,
      finalSelectedAnimePaths.length > 0
        ? {
            facet: "anime",
            libraryId: defaultLibraryIdForFacet("anime"),
            label: t("setup.facetAnime"),
          }
        : null,
    ].filter(
      (
        value,
      ): value is {
        facet: "movie" | "series" | "anime";
        libraryId: string;
        label: string;
      } => value !== null,
    );

    try {
      await finalizeImportedMonitorSnapshots();
      const { data, error } = await client
        .mutation(completeSetupMutation, {})
        .toPromise();
      if (error) {
        throw error;
      }
      if (!data?.completeSetup?.completed) {
        throw new Error(t("setup.importFinalizeFailed"));
      }

      await Promise.all(
        selectedFacets.map(async ({ libraryId, label }) => {
          try {
            const result = await client
              .mutation(scanLibraryMutation, {
                input: {
                  libraryId,
                  importWarmupSessionId: importWarmupProgress?.sessionId ?? null,
                },
              })
              .toPromise();
            if (result.error) throw result.error;
          } catch (error) {
            const message =
              error instanceof Error ? error.message : String(error ?? "");
            if (/library scan already running/i.test(message)) {
              toast.info(
                t("settings.libraryScanAlreadyRunning").replace(
                  "{{facet}}",
                  label,
                ),
              );
              return;
            }

            toast.warning(message || t("settings.libraryScanFailed"));
          }
        }),
      );
      navigateAfterSetup();
    } catch (error) {
      toast.warning(
        error instanceof Error
          ? error.message
          : t("setup.importFinalizeFailed"),
      );
    } finally {
      setFinishingAction(null);
    }
  }, [
    client,
    finalSelectedAnimePaths.length,
    finalSelectedMoviesPaths.length,
    finalSelectedSeriesPaths.length,
    finalizeImportedMonitorSnapshots,
    importWarmupProgress?.sessionId,
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
          : importedPaths.filter(
              (entry) => prev.includes(entry) || entry === path,
            ),
      );
    },
    [],
  );

  const toggleMoviesPath = useCallback(
    (path: string) => {
      const importedPaths =
        importPreview?.rootFolders
          .filter((folder) => folder.source === "radarr")
          .map((folder) => folder.path) ?? [];
      toggleImportedPathSelection(setSelectedMoviesPaths, path, importedPaths);
    },
    [importPreview, toggleImportedPathSelection],
  );

  const toggleSeriesPath = useCallback(
    (path: string) => {
      toggleImportedPathSelection(
        setSelectedSeriesPaths,
        path,
        importedSonarrPaths,
      );
    },
    [importedSonarrPaths, toggleImportedPathSelection],
  );

  const toggleAnimePath = useCallback(
    (path: string) => {
      toggleImportedPathSelection(
        setSelectedAnimePaths,
        path,
        importedSonarrPaths,
      );
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
            : importedPaths.filter(
                (entry) => prev.includes(entry) || entry === trimmed,
              ),
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
      const importedPaths =
        importPreview?.rootFolders
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

  const setDcPassword = useCallback((dedupKey: string, password: string) => {
    setDcPasswordOverrides((prev) => {
      const next = new Map(prev);
      if (password) next.set(dedupKey, password);
      else next.delete(dedupKey);
      return next;
    });
  }, []);

  const setIdxApiKey = useCallback((dedupKey: string, apiKey: string) => {
    setIdxApiKeyOverrides((prev) => {
      const next = new Map(prev);
      if (apiKey) next.set(dedupKey, apiKey);
      else next.delete(dedupKey);
      return next;
    });
  }, []);

  // ── Render ──────────────────────────────────────────────────────────

  // Step mapping for progress bar (step 0 = welcome, not shown in bar)
  const progressStep = currentStep > 0 ? currentStep - 1 : -1;
  const isWideImportStep =
    currentStep === 0 ||
    (wizardPath === "import" && (currentStep === 1 || currentStep === 2));

  return (
    <div
      className={`mx-auto flex min-h-screen w-full flex-col items-center justify-center px-4 py-10 ${
        isWideImportStep ? "max-w-6xl" : "max-w-2xl"
      }`}
    >
      <div className="mb-8 flex items-center gap-2.5">
        <ScryerLogo className="h-9 w-9" />
        <span className="font-[var(--font-space-grotesk)] text-lg font-bold tracking-tight text-[var(--scry-ink2)]">
          Scryer
        </span>
      </div>

      {currentStep > 0 && (
        <div className="mb-8 w-full">
          <SetupProgressBar
            currentStep={progressStep}
            stepLabels={stepLabels}
          />
        </div>
      )}

      {/* ── Step 0: Welcome (shared) ─────────────────────────────────── */}
      {currentStep === 0 && (
        <SetupWelcomeView
          t={t}
          onFreshSetup={() => goToStep(1, "fresh")}
          onImportSetup={() => goToStep(1, "import")}
          onRestoreSetup={() => goToStep(1, "restore")}
          onSkip={finishSetup}
          skipping={finishing}
          canRestoreSetup={canRestoreSetup}
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
          localPathStyle={dcLocalPathStyle}
          onDraftChange={handleDcDraftChange}
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

      {currentStep === 1 && wizardPath === "restore" && canRestoreSetup && (
        <SetupRestoreView
          t={t}
          onBack={() => goToStep(0)}
          onBackendRestarting={onBackendRestarting}
        />
      )}

      {currentStep === 1 && wizardPath === "import" && (
        <SetupImportConnectView
          t={t}
          sonarrUrl={sonarrUrl}
          sonarrApiKey={sonarrApiKey}
          radarrUrl={radarrUrl}
          radarrApiKey={radarrApiKey}
          prowlarrUrl={prowlarrUrl}
          prowlarrApiKey={prowlarrApiKey}
          onSonarrUrlChange={setSonarrUrl}
          onSonarrApiKeyChange={setSonarrApiKey}
          onRadarrUrlChange={setRadarrUrl}
          onRadarrApiKeyChange={setRadarrApiKey}
          onProwlarrUrlChange={setProwlarrUrl}
          onProwlarrApiKeyChange={setProwlarrApiKey}
          onConnect={handleImportConnect}
          onBack={() => goToStep(0)}
          connecting={importConnecting}
          error={importConnectError}
          sonarrError={importConnectServiceErrors.sonarr}
          radarrError={importConnectServiceErrors.radarr}
          prowlarrError={importConnectServiceErrors.prowlarr}
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
          dcPasswordOverrides={dcPasswordOverrides}
          idxApiKeyOverrides={idxApiKeyOverrides}
          indexerProviderConfigFieldsByType={indexerProviderConfigFieldsByType}
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
          onSetDcPassword={setDcPassword}
          onSetIdxApiKey={setIdxApiKey}
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
          monitorWarmupProgress={importWarmupProgress}
          monitorWarmupError={importWarmupError}
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
