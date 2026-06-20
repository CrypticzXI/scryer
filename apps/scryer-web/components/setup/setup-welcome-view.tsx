import { Rocket, ArrowRightLeft, Upload } from "lucide-react";
import { Card, CardContent } from "@/components/ui/card";

interface SetupWelcomeViewProps {
  t: (key: string) => string;
  onFreshSetup: () => void;
  onImportSetup: () => void;
  onRestoreSetup: () => void;
  onSkip: () => void;
  skipping: boolean;
  canRestoreSetup: boolean;
}

export function SetupWelcomeView({
  t,
  onFreshSetup,
  onImportSetup,
  onRestoreSetup,
  onSkip,
  skipping,
  canRestoreSetup,
}: SetupWelcomeViewProps) {
  return (
    <div id="setup-welcome-view" className="flex flex-col items-center gap-8">
      <div className="text-center">
        <h1
          className="mb-3 text-3xl font-bold tracking-tight"
          style={{ fontFamily: "'Space Grotesk Variable', 'Inter Variable', ui-sans-serif, system-ui, sans-serif" }}
        >
          {t("setup.welcomeTitle")}
        </h1>
        <p className="text-muted-foreground">{t("setup.welcomeDescription")}</p>
      </div>
      <div
        className={`grid w-full gap-4 ${
          canRestoreSetup ? "max-w-5xl xl:grid-cols-3" : "max-w-3xl md:grid-cols-2"
        }`}
      >
        <Card
          id="setup-welcome-fresh"
          className="cursor-pointer transition-colors hover:border-primary"
          onClick={onFreshSetup}
        >
          <CardContent className="flex flex-col items-center gap-3 p-6 text-center">
            <Rocket className="h-8 w-8 text-emerald-500" />
            <div>
              <p className="font-semibold">{t("setup.freshSetup")}</p>
              <p className="mt-1 text-sm text-muted-foreground">
                {t("setup.freshSetupDescription")}
              </p>
            </div>
          </CardContent>
        </Card>
        <Card
          id="setup-welcome-import"
          className="cursor-pointer transition-colors hover:border-primary"
          onClick={onImportSetup}
        >
          <CardContent className="flex flex-col items-center gap-3 p-6 text-center">
            <ArrowRightLeft className="h-8 w-8 text-blue-500" />
            <div>
              <p className="font-semibold">{t("setup.importSetup")}</p>
              <p className="mt-1 text-sm text-muted-foreground">
                {t("setup.importSetupDescription")}
              </p>
            </div>
          </CardContent>
        </Card>
        {canRestoreSetup ? (
          <Card
            id="setup-welcome-restore"
            className="cursor-pointer transition-colors hover:border-primary"
            onClick={onRestoreSetup}
          >
            <CardContent className="flex flex-col items-center gap-3 p-6 text-center">
              <Upload className="h-8 w-8 text-violet-500" />
              <div>
                <p className="font-semibold">{t("setup.restoreSetup")}</p>
                <p className="mt-1 text-sm text-muted-foreground">
                  {t("setup.restoreSetupDescription")}
                </p>
              </div>
            </CardContent>
          </Card>
        ) : null}
      </div>
      <button
        id="setup-welcome-skip"
        type="button"
        onClick={onSkip}
        disabled={skipping}
        className="text-sm text-muted-foreground underline-offset-4 hover:underline"
      >
        {skipping ? t("setup.skipping") : t("setup.skipSetup")}
      </button>
    </div>
  );
}
