import { useState } from "react";
import { FolderOpen } from "lucide-react";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { FolderBrowserDialog } from "./folder-browser-dialog";
import {
  SetupBackButton,
  SetupPanel,
  SetupPrimaryButton,
  SetupStepHeader,
} from "./setup-chrome";

interface SetupMediaPathsViewProps {
  t: (key: string) => string;
  moviesPath: string;
  seriesPath: string;
  animePath: string;
  onMoviesPathChange: (value: string) => void;
  onSeriesPathChange: (value: string) => void;
  onAnimePathChange: (value: string) => void;
  onNext: () => void;
  onBack: () => void;
  onSkip?: () => void;
  saving: boolean;
  error: string | null;
}

type BrowseTarget = "movies" | "series" | "anime" | null;

export function SetupMediaPathsView({
  t,
  moviesPath,
  seriesPath,
  animePath,
  onMoviesPathChange,
  onSeriesPathChange,
  onAnimePathChange,
  onNext,
  onBack,
  onSkip,
  saving,
  error,
}: SetupMediaPathsViewProps) {
  const [browseTarget, setBrowseTarget] = useState<BrowseTarget>(null);
  const canProceed = true;

  const browseInitialPath =
    browseTarget === "movies"
      ? moviesPath
      : browseTarget === "series"
        ? seriesPath
        : browseTarget === "anime"
          ? animePath
          : "/";

  function handleBrowseSelect(path: string) {
    if (browseTarget === "movies") onMoviesPathChange(path);
    else if (browseTarget === "series") onSeriesPathChange(path);
    else if (browseTarget === "anime") onAnimePathChange(path);
  }

  return (
    <SetupPanel id="setup-media-paths-view" className="flex flex-col gap-6">
      <SetupStepHeader
        icon={FolderOpen}
        title={t("setup.mediaPathsTitle")}
        subtitle={t("setup.mediaPathsDescription")}
      />
      <div className="mx-auto flex w-full max-w-md flex-col gap-4">
        <div className="space-y-2">
          <Label htmlFor="setup-media-paths-movies-path">
            {t("setup.moviesPath")}
            <span className="ml-1.5 text-xs font-normal text-muted-foreground">
              {t("setup.optional")}
            </span>
          </Label>
          <div className="flex gap-2">
            <Input
              id="setup-media-paths-movies-path"
              value={moviesPath}
              onChange={(e) => onMoviesPathChange(e.target.value)}
              placeholder="/data/movies"
              className="font-[var(--font-code)]"
            />
            <IconButton
              id="setup-media-paths-movies-browse"
              label={t("setup.browse")}
              tone="neutral"
              onClick={() => setBrowseTarget("movies")}
            >
              <FolderOpen className="h-4 w-4" />
            </IconButton>
          </div>
        </div>
        <div className="space-y-2">
          <Label htmlFor="setup-media-paths-series-path">
            {t("setup.seriesPath")}
            <span className="ml-1.5 text-xs font-normal text-muted-foreground">
              {t("setup.optional")}
            </span>
          </Label>
          <div className="flex gap-2">
            <Input
              id="setup-media-paths-series-path"
              value={seriesPath}
              onChange={(e) => onSeriesPathChange(e.target.value)}
              placeholder="/data/series"
              className="font-[var(--font-code)]"
            />
            <IconButton
              id="setup-media-paths-series-browse"
              label={t("setup.browse")}
              tone="neutral"
              onClick={() => setBrowseTarget("series")}
            >
              <FolderOpen className="h-4 w-4" />
            </IconButton>
          </div>
        </div>
        <div className="space-y-2">
          <Label htmlFor="setup-media-paths-anime-path">
            {t("setup.animePath")}
            <span className="ml-1.5 text-xs font-normal text-muted-foreground">
              {t("setup.optional")}
            </span>
          </Label>
          <div className="flex gap-2">
            <Input
              id="setup-media-paths-anime-path"
              value={animePath}
              onChange={(e) => onAnimePathChange(e.target.value)}
              placeholder="/data/anime"
              className="font-[var(--font-code)]"
            />
            <IconButton
              id="setup-media-paths-anime-browse"
              label={t("setup.browse")}
              tone="neutral"
              onClick={() => setBrowseTarget("anime")}
            >
              <FolderOpen className="h-4 w-4" />
            </IconButton>
          </div>
        </div>
        {error && (
          <p id="setup-media-paths-error" data-ui="setup-media-paths-error" className="text-sm text-destructive">
            {error}
          </p>
        )}
      </div>
      <div className="flex items-center justify-between pt-2">
        <SetupBackButton id="setup-media-paths-back" onClick={onBack}>
          {t("setup.back")}
        </SetupBackButton>
        <div className="flex items-center gap-3">
          {onSkip && (
            <Button id="setup-media-paths-skip" type="button" variant="link" onClick={onSkip}>
              {t("setup.skip")}
            </Button>
          )}
          <SetupPrimaryButton id="setup-media-paths-next" onClick={onNext} disabled={!canProceed || saving}>
            {saving ? t("label.saving") : t("setup.next")}
          </SetupPrimaryButton>
        </div>
      </div>

      <FolderBrowserDialog
        open={browseTarget !== null}
        onOpenChange={(open) => { if (!open) setBrowseTarget(null); }}
        onSelect={handleBrowseSelect}
        initialPath={browseInitialPath || "/"}
        title={t("setup.browse")}
      />
    </SetupPanel>
  );
}
