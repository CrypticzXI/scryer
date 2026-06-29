import { useCallback, useEffect, useState } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";
import { useClient } from "urql";

import {
  qualityProfilesInitQuery,
  setupStatusQuery,
  setupWizardProviderTypesInitQuery,
} from "@/lib/graphql/queries";
import {
  saveQualityProfileSettingsMutation,
  updateLibraryPathsMutation,
  completeSetupMutation,
} from "@/lib/graphql/mutations";
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
import type { ProviderTypeInfo } from "@/lib/types";

import ScryerLogo from "@/components/scryer-logo";
import { SetupProgressBar } from "./setup-progress-bar";
import { SetupWelcomeView } from "./setup-welcome-view";
import { SetupPersonaView } from "./setup-persona-view";
import { SetupMediaPathsView } from "./setup-media-paths-view";
import { SetupDownloadClientView } from "./setup-download-client-view";
import { SetupIndexerView } from "./setup-indexer-view";
import { SetupSummaryView } from "./setup-summary-view";
import SetupImportWizard from "./setup-import-wizard";
import { SetupPluginsView } from "./setup-plugins-view";
import { SetupRestoreView } from "./setup-restore-view";

const FALLBACK_PROVIDER_OPTIONS: SetupIndexerProviderOption[] = [];

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

  const tImport = useCallback(
    (key: string, values?: Record<string, unknown>) =>
      t(
        key,
        values as
          | Record<string, string | number | boolean | null | undefined>
          | undefined,
      ),
    [t],
  );

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

  // Widened adapter so the import wizard's `(step, path?: string)` signature
  // satisfies the container's narrower path union (it only ever passes
  // "import" or undefined).
  const goToImportStep = useCallback(
    (step: number, path?: string) =>
      goToStep(step, path as "fresh" | "import" | "restore" | undefined),
    [goToStep],
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
    handleIdxNameChange,
    handleIdxProviderTypeChange,
    handleIdxConfigValueChange,
    testIndexer,
    handleIdxTestAndSave,
  } = useIndexerSetup({ client, t });

  // ── Summary / Finish (fresh path) ───────────────────────────────────
  const [finishingAction, setFinishingAction] = useState<"finish" | null>(null);
  const finishing = finishingAction !== null;

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
          t("setup.stepLibraries"),
          t("setup.stepQuality"),
          t("setup.stepSources"),
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

  // ── Complete setup ──────────────────────────────────────────────────
  const navigateAfterSetup = useCallback(() => {
    navigate(isReentry ? "/settings" : "/movies", { replace: true });
  }, [isReentry, navigate]);

  const finishSetup = useCallback(async () => {
    setFinishingAction("finish");
    try {
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
    } catch {
      navigateAfterSetup();
    } finally {
      setFinishingAction(null);
    }
  }, [client, navigateAfterSetup, t]);

  // ── Render ──────────────────────────────────────────────────────────

  // Step mapping for progress bar (step 0 = welcome, not shown in bar)
  const progressStep = currentStep > 0 ? currentStep - 1 : -1;
  const isWideImportStep =
    currentStep === 0 ||
    (wizardPath === "import" && (currentStep === 1 || currentStep === 2));
  const isPersonaStep = wizardPath === "fresh" && currentStep === 1;
  const shellMaxWidth = isWideImportStep
    ? "max-w-6xl"
    : isPersonaStep
      ? "max-w-3xl"
      : "max-w-2xl";

  return (
    <div
      className={`mx-auto flex min-h-screen w-full flex-col items-center justify-center px-4 py-10 ${shellMaxWidth}`}
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
            onStepClick={(i) => goToStep(i + 1, wizardPath)}
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

      {wizardPath === "import" && currentStep >= 1 && (
        <SetupImportWizard
          client={client}
          t={tImport}
          currentStep={currentStep}
          goToStep={goToImportStep}
          onExit={navigateAfterSetup}
        />
      )}
    </div>
  );
}
