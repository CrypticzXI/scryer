import { useCallback, useEffect, useRef } from "react";
import type { Client } from "urql";
import {
  ArrowLeft,
  ArrowLeftRight,
  ArrowRight,
  BadgeCheck,
  Check,
  FolderTree,
  type LucideIcon,
  Plug,
  Radar,
  SlidersHorizontal,
} from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { useExternalImportSetup } from "@/lib/hooks/use-external-import-setup";

import { SETUP_PRIMARY_CTA, SetupPanel, SetupStepHeader } from "./setup-chrome";
import SetupImportConnectView from "./setup-import-connect-view";
import SetupImportLibrariesView from "./setup-import-libraries-view";
import SetupImportQualityView from "./setup-import-quality-view";
import SetupImportSourcesView from "./setup-import-sources-view";
import SetupImportSummaryView from "./setup-import-summary-view";

type WizardTranslate = (
  key: string,
  values?: Record<string, unknown>,
) => string;

interface SetupImportWizardProps {
  client: Client;
  t: WizardTranslate;
  /** Current import step index: 1=connect, 2=libraries, 3=quality, 4=sources, 5=summary. */
  currentStep: number;
  goToStep: (step: number, path?: string) => void;
  /** Navigate away once the import is finished (settings vs library home). */
  onExit: () => void;
}

interface StepChrome {
  icon: LucideIcon;
  titleKey: string;
  subtitleKey: string;
}

const STEP_CHROME: Record<number, StepChrome> = {
  1: {
    icon: ArrowLeftRight,
    titleKey: "setup.connectMultiTitle",
    subtitleKey: "setup.connectMultiDescription",
  },
  2: {
    icon: FolderTree,
    titleKey: "setup.librariesTitle",
    subtitleKey: "setup.librariesDescription",
  },
  3: {
    icon: SlidersHorizontal,
    titleKey: "setup.qualityPersonaTitle",
    subtitleKey: "setup.qualityPersonaDescription",
  },
  4: {
    icon: Plug,
    titleKey: "setup.sourcesTitle",
    subtitleKey: "setup.sourcesDescription",
  },
  5: {
    icon: BadgeCheck,
    titleKey: "setup.importReadyTitle",
    subtitleKey: "setup.importReadyDescription",
  },
};

/**
 * Owns the entire multi-instance import path (Connect → Libraries → Quality →
 * Sources → Summary): the step chrome, navigation, and the
 * execute/finalize/scan orchestration. The container renders the brand row +
 * stepper and delegates here for `wizardPath === "import"`.
 */
export function SetupImportWizard({
  client,
  t,
  currentStep,
  goToStep,
  onExit,
}: SetupImportWizardProps) {
  const wizard = useExternalImportSetup({ client });
  const {
    loadPreview,
    pollAggregateProgress,
    warmupSessionLost,
    recoverLostWarmup,
    pendingReverify,
    clearPendingReverify,
    instances,
    verifyInstance,
  } = wizard;

  // Keep the latest preview in a ref so the polling loop below can read it
  // without re-subscribing (avoids a refetch-on-every-update tight loop).
  const previewRef = useRef(wizard.preview);
  useEffect(() => {
    previewRef.current = wizard.preview;
  }, [wizard.preview]);

  // On the Libraries step, (re)load the preview until every connected arr
  // source's warmup has settled — root folders can lag the initial warmup
  // start, so a single fetch would leave the mapping tray empty.
  const arrSessionCount = wizard.connectedArrSessionIds.length;
  useEffect(() => {
    if (currentStep !== 2) return;
    // Prowlarr-only (no arr warmups): one fetch, nothing to poll for.
    if (arrSessionCount === 0) {
      void loadPreview();
      return;
    }
    const settled = () => {
      const sources = previewRef.current?.arrSources ?? [];
      return (
        sources.length > 0 &&
        sources.every(
          (s) =>
            s.status === "completed" ||
            s.status === "failed" ||
            s.status === "canceled",
        )
      );
    };
    void loadPreview();
    const id = setInterval(() => {
      if (settled()) {
        clearInterval(id);
        return;
      }
      void loadPreview();
    }, 3000);
    return () => clearInterval(id);
  }, [currentStep, loadPreview, arrSessionCount]);

  // Steps after Libraries rely on the preview-derived root mappings, but the
  // preview is in-memory while the step lives in the URL. Two cases must keep it
  // in sync, or finalize sends an incomplete root set (backend then rejects
  // "missing mapping for source … root …"):
  //   • a refresh restores a later step with no preview at all;
  //   • a Retry warms NEW sessions whose final root set (title_root_paths) only
  //     lands once the warmup completes.
  // So: load it whenever it's missing, and refresh it once the warmup completes
  // for the current session set (tracked by signature to load at most once).
  const previewSyncedSigRef = useRef<string | null>(null);
  useEffect(() => {
    if (currentStep < 3 || arrSessionCount === 0) return;
    const sig = wizard.connectedArrSessionIds.join("|");
    if (!wizard.preview) {
      if (wizard.warmupComplete) previewSyncedSigRef.current = sig;
      void loadPreview();
      return;
    }
    if (wizard.warmupComplete && previewSyncedSigRef.current !== sig) {
      previewSyncedSigRef.current = sig;
      void loadPreview();
    }
  }, [
    currentStep,
    arrSessionCount,
    wizard.preview,
    wizard.warmupComplete,
    wizard.connectedArrSessionIds,
    loadPreview,
  ]);

  // Poll aggregated warmup progress while on the Summary step until it reaches a
  // terminal state (completed OR failed/canceled) — otherwise a failed warmup
  // would re-poll forever.
  useEffect(() => {
    if (currentStep !== 5) return;
    void pollAggregateProgress();
    if (wizard.warmupSettled) return;
    const id = setInterval(() => {
      void pollAggregateProgress();
    }, 3000);
    return () => clearInterval(id);
  }, [currentStep, wizard.warmupSettled, pollAggregateProgress]);

  // A lost warmup session (pruned after a restart / TTL) can't be recovered in
  // place — no retry or preview reload will work against a dead session. Force
  // the user back to Connect from whatever step surfaced it, resetting the
  // connections so re-verifying mints fresh sessions.
  useEffect(() => {
    if (!warmupSessionLost || currentStep <= 1) return;
    recoverLostWarmup();
    goToStep(1, "import");
    toast.warning(t("setup.importWarmupSessionExpired"));
  }, [warmupSessionLost, currentStep, recoverLostWarmup, goToStep, t]);

  // After that recovery lands on Connect, auto-verify the restored connections
  // once so a fresh warmup session starts without the operator having to
  // manually re-blur each field. Instances missing a usable key are left for
  // manual re-entry. Gated on pendingReverify so it never fires mid-typing.
  useEffect(() => {
    if (currentStep !== 1 || !pendingReverify) return;
    clearPendingReverify();
    for (const inst of instances) {
      if (
        /^https?:\/\/.+/.test(inst.baseUrl.trim()) &&
        inst.apiKey.trim().length >= 6
      ) {
        void verifyInstance(inst.id);
      }
    }
  }, [
    currentStep,
    pendingReverify,
    clearPendingReverify,
    instances,
    verifyInstance,
  ]);

  const goConnectContinue = useCallback(async () => {
    await loadPreview();
    goToStep(2, "import");
  }, [loadPreview, goToStep]);

  const goSourcesContinue = useCallback(async () => {
    const { ok, error } = await wizard.executeSources();
    if (!ok) {
      toast.warning(error ?? t("setup.connectionFailed"));
      return;
    }
    goToStep(5, "import");
  }, [wizard, t, goToStep]);

  const finish = useCallback(async () => {
    const { ok, scanErrors, error } = await wizard.finalizeImport();
    if (!ok) {
      toast.warning(error ?? t("setup.importFinalizeFailed"));
      return;
    }
    for (const message of scanErrors) {
      toast.info(message);
    }
    onExit();
  }, [wizard, t, onExit]);

  // A lost warmup session can't be re-fetched — reset connections and route back
  // to Connect, where re-verifying mints fresh sessions.
  const reconnect = useCallback(() => {
    wizard.recoverLostWarmup();
    goToStep(1, "import");
  }, [wizard, goToStep]);

  const chrome = STEP_CHROME[currentStep];
  if (!chrome) return null;

  const back = () => goToStep(currentStep <= 1 ? 0 : currentStep - 1, "import");

  let body: React.ReactNode = null;
  let primaryLabel = t("setup.continue");
  let primaryIcon: LucideIcon = ArrowRight;
  let primaryDisabled = false;
  let onPrimary: () => void = () => goToStep(currentStep + 1, "import");

  switch (currentStep) {
    case 1:
      body = <SetupImportConnectView wizard={wizard} t={t} />;
      primaryLabel = t("setup.connectAndScan");
      primaryIcon = Radar;
      primaryDisabled = !wizard.canLeaveConnect;
      onPrimary = () => void goConnectContinue();
      break;
    case 2:
      body = <SetupImportLibrariesView wizard={wizard} t={t} />;
      // Block Continue until warmups have settled (roots discovered) AND every
      // warmed root is mapped with no blank manual root — mirrors the backend's
      // "every warmed root must be mapped" finalize rule.
      primaryDisabled = !wizard.previewSettled || !wizard.mappingReady;
      onPrimary = () => goToStep(3, "import");
      break;
    case 3:
      body = <SetupImportQualityView wizard={wizard} t={t} />;
      // Can't proceed until every mapped library has a quality profile.
      primaryDisabled = !wizard.qualityReady;
      onPrimary = () => goToStep(4, "import");
      break;
    case 4:
      body = <SetupImportSourcesView wizard={wizard} t={t} />;
      // Block until every selected client/indexer that needs a user-supplied
      // secret has one (otherwise execute creates non-functional configs).
      primaryDisabled = wizard.executing || !wizard.sourcesReady;
      onPrimary = () => void goSourcesContinue();
      break;
    case 5:
      body = (
        <SetupImportSummaryView wizard={wizard} t={t} onReconnect={reconnect} />
      );;
      primaryLabel = t("setup.finishImport");
      primaryIcon = Check;
      // Finish needs the warmup complete AND a loaded preview whose every
      // detected root is mapped — otherwise finalize would omit required
      // source-root mappings (e.g. after a refresh that dropped the preview).
      primaryDisabled =
        !wizard.warmupComplete ||
        !wizard.previewSettled ||
        !wizard.mappingReady ||
        wizard.finalizing;
      onPrimary = () => void finish();
      break;
  }

  const PrimaryIcon = primaryIcon;

  return (
    <SetupPanel id="setup-import-step">
      <SetupStepHeader
        icon={chrome.icon}
        title={t(chrome.titleKey)}
        subtitle={t(chrome.subtitleKey)}
      />

      <div className="mt-6">{body}</div>

      <div className="mt-6 flex items-center justify-between border-t border-[var(--scry-hover)] pt-5">
        <Button
          type="button"
          variant="ghost"
          onClick={back}
          className="text-[var(--scry-ink2)]"
          // Spec: Back is hidden (but still occupies space) on Connect + Libraries.
          style={{ visibility: currentStep <= 2 ? "hidden" : "visible" }}
          tabIndex={currentStep <= 2 ? -1 : undefined}
          aria-hidden={currentStep <= 2 || undefined}
        >
          <ArrowLeft className="h-4 w-4" />
          {t("setup.back")}
        </Button>
        <Button
          type="button"
          onClick={onPrimary}
          disabled={primaryDisabled}
          className={`${SETUP_PRIMARY_CTA} shadow-none`}
        >
          {primaryLabel}
          <PrimaryIcon className="h-4 w-4" />
        </Button>
      </div>
    </SetupPanel>
  );
}

export default SetupImportWizard;
