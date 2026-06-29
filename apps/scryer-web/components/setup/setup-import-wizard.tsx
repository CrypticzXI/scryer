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
  const { loadPreview, pollAggregateProgress } = wizard;

  // Keep the latest preview in a ref so the polling loop below can read it
  // without re-subscribing (avoids a refetch-on-every-update tight loop).
  const previewRef = useRef(wizard.preview);
  useEffect(() => {
    previewRef.current = wizard.preview;
  }, [wizard.preview]);

  // On the Libraries step, (re)load the preview until every connected arr
  // source's warmup has settled — root folders can lag the initial warmup
  // start, so a single fetch would leave the mapping tray empty.
  useEffect(() => {
    if (currentStep !== 2) return;
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
  }, [currentStep, loadPreview]);

  // Poll aggregated warmup progress while on the Summary step until complete.
  useEffect(() => {
    if (currentStep !== 5) return;
    void pollAggregateProgress();
    if (wizard.warmupComplete) return;
    const id = setInterval(() => {
      void pollAggregateProgress();
    }, 3000);
    return () => clearInterval(id);
  }, [currentStep, wizard.warmupComplete, pollAggregateProgress]);

  const goConnectContinue = useCallback(async () => {
    await loadPreview();
    goToStep(2, "import");
  }, [loadPreview, goToStep]);

  const goSourcesContinue = useCallback(async () => {
    const ok = await wizard.executeSources();
    if (!ok) {
      toast.warning(wizard.executeError ?? t("setup.connectionFailed"));
      return;
    }
    goToStep(5, "import");
  }, [wizard, t, goToStep]);

  const finish = useCallback(async () => {
    const { ok, scanErrors } = await wizard.finalizeImport();
    if (!ok) {
      toast.warning(wizard.finalizeError ?? t("setup.importFinalizeFailed"));
      return;
    }
    for (const message of scanErrors) {
      toast.info(message);
    }
    onExit();
  }, [wizard, t, onExit]);

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
      // Backend finalize requires every warmed root mapped (and no blank manual
      // root), so block Continue until the board is complete.
      primaryDisabled = !wizard.mappingReady;
      onPrimary = () => goToStep(3, "import");
      break;
    case 3:
      body = <SetupImportQualityView wizard={wizard} t={t} />;
      onPrimary = () => goToStep(4, "import");
      break;
    case 4:
      body = <SetupImportSourcesView wizard={wizard} t={t} />;
      primaryDisabled = wizard.executing;
      onPrimary = () => void goSourcesContinue();
      break;
    case 5:
      body = <SetupImportSummaryView wizard={wizard} t={t} />;
      primaryLabel = t("setup.finishImport");
      primaryIcon = Check;
      primaryDisabled = !wizard.warmupComplete || wizard.finalizing;
      onPrimary = () => void finish();
      break;
  }

  const PrimaryIcon = primaryIcon;
  const footNote =
    currentStep === 1 ? t("setup.connectReadOnlyNote") : null;

  return (
    <SetupPanel id="setup-import-step">
      <SetupStepHeader
        icon={chrome.icon}
        title={t(chrome.titleKey)}
        subtitle={t(chrome.subtitleKey)}
      />

      <div className="mt-6">{body}</div>

      {footNote ? (
        <p className="mt-5 text-center text-xs text-[var(--scry-muted3)]">
          {footNote}
        </p>
      ) : null}

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
          className={SETUP_PRIMARY_CTA}
        >
          {primaryLabel}
          <PrimaryIcon className="h-4 w-4" />
        </Button>
      </div>
    </SetupPanel>
  );
}

export default SetupImportWizard;
