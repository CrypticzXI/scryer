import * as React from "react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useTranslate } from "@/lib/context/translate-context";
import type { TitleRecord } from "@/lib/types";
import type { LibraryRootRecord } from "@/lib/types/titles";
import type { ParsedQualityProfile } from "@/lib/types/quality-profiles";
import type { TitleOptionUpdates } from "@/lib/types/title-options";

const UNCHANGED_VALUE = "__unchanged__";
const INHERIT_VALUE = "__inherit__";
const ENABLED_VALUE = "enabled";
const DISABLED_VALUE = "disabled";

type DraftState = {
  qualityProfileId: string;
  rootFolderId: string;
  monitorType: string;
  useSeasonFolders: string;
  monitorSpecials: string;
  interSeasonMovies: string;
  fillerPolicy: string;
  recapPolicy: string;
};

type BulkTitleEditDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  view: string;
  selectedTitles: TitleRecord[];
  qualityProfiles: ParsedQualityProfile[];
  rootFolders: LibraryRootRecord[];
  busy: boolean;
  onSubmit: (changes: TitleOptionUpdates) => Promise<void> | void;
};

function initialDraftState(): DraftState {
  return {
    qualityProfileId: UNCHANGED_VALUE,
    rootFolderId: UNCHANGED_VALUE,
    monitorType: UNCHANGED_VALUE,
    useSeasonFolders: UNCHANGED_VALUE,
    monitorSpecials: UNCHANGED_VALUE,
    interSeasonMovies: UNCHANGED_VALUE,
    fillerPolicy: UNCHANGED_VALUE,
    recapPolicy: UNCHANGED_VALUE,
  };
}

export function BulkTitleEditDialog({
  open,
  onOpenChange,
  view,
  selectedTitles,
  qualityProfiles,
  rootFolders,
  busy,
  onSubmit,
}: BulkTitleEditDialogProps) {
  const t = useTranslate();
  const [draft, setDraft] = React.useState<DraftState>(initialDraftState);

  const isMovieView = view === "movies";
  const isAnimeView = view === "anime";
  const hasPendingChange = Object.values(draft).some(
    (value) => value !== UNCHANGED_VALUE,
  );
  const folderLabel = React.useCallback(
    (path: string) => path.split("/").filter(Boolean).pop() ?? path,
    [],
  );
  const sortedRootFolders = React.useMemo(
    () =>
      [...rootFolders].sort((left, right) => {
        if (left.isDefault !== right.isDefault) {
          return left.isDefault ? -1 : 1;
        }
        return left.path.localeCompare(right.path);
      }),
    [rootFolders],
  );

  React.useEffect(() => {
    if (!open) {
      return;
    }
    setDraft(initialDraftState());
  }, [open]);

  const monitorOptions = React.useMemo(
    () =>
      isMovieView
        ? [
            {
              value: "MONITORED",
              label: t("search.monitorType.monitored"),
            },
            {
              value: "UNMONITORED",
              label: t("search.monitorType.unmonitored"),
            },
          ]
        : [
            {
              value: "FUTURE_EPISODES",
              label: t("search.monitorType.futureEpisodes"),
            },
            {
              value: "MISSING_AND_FUTURE_EPISODES",
              label: t("search.monitorType.missingAndFutureEpisodes"),
            },
            {
              value: "ALL_EPISODES",
              label: t("search.monitorType.allEpisodes"),
            },
            {
              value: "NONE",
              label: t("search.monitorType.none"),
            },
          ],
    [isMovieView, t],
  );

  const buildChanges = React.useCallback((): TitleOptionUpdates => {
    const changes: TitleOptionUpdates = {};
    if (draft.qualityProfileId !== UNCHANGED_VALUE) {
      changes.qualityProfileId =
        draft.qualityProfileId === INHERIT_VALUE ? "" : draft.qualityProfileId;
    }
    if (draft.rootFolderId !== UNCHANGED_VALUE) {
      changes.rootFolderId = draft.rootFolderId;
    }
    if (draft.monitorType !== UNCHANGED_VALUE) {
      changes.monitorType = draft.monitorType;
    }
    if (draft.useSeasonFolders !== UNCHANGED_VALUE) {
      changes.useSeasonFolders = draft.useSeasonFolders === ENABLED_VALUE;
    }
    if (draft.monitorSpecials !== UNCHANGED_VALUE) {
      changes.monitorSpecials = draft.monitorSpecials === ENABLED_VALUE;
    }
    if (draft.interSeasonMovies !== UNCHANGED_VALUE) {
      changes.interSeasonMovies = draft.interSeasonMovies === ENABLED_VALUE;
    }
    if (draft.fillerPolicy !== UNCHANGED_VALUE) {
      changes.fillerPolicy =
        draft.fillerPolicy === INHERIT_VALUE ? "" : draft.fillerPolicy;
    }
    if (draft.recapPolicy !== UNCHANGED_VALUE) {
      changes.recapPolicy =
        draft.recapPolicy === INHERIT_VALUE ? "" : draft.recapPolicy;
    }
    return changes;
  }, [draft]);

  const handleSubmit = React.useCallback(() => {
    if (!hasPendingChange || busy) {
      return;
    }
    void Promise.resolve(onSubmit(buildChanges()));
  }, [buildChanges, busy, hasPendingChange, onSubmit]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle>{t("title.bulkEditTitle")}</DialogTitle>
          <DialogDescription>
            {t("title.bulkEditDescription", { count: selectedTitles.length })}
          </DialogDescription>
        </DialogHeader>

        <div className="grid gap-4 md:grid-cols-2">
          <EditableField label={t("settings.qualityProfileSection")}>
            <Select
              value={draft.qualityProfileId}
              onValueChange={(value) =>
                setDraft((previous) => ({ ...previous, qualityProfileId: value }))
              }
              disabled={busy}
            >
              <SelectTrigger className="h-9 w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={UNCHANGED_VALUE}>
                  {t("label.unchanged")}
                </SelectItem>
                <SelectItem value={INHERIT_VALUE}>
                  {t("title.inheritDefault")}
                </SelectItem>
                {qualityProfiles.map((profile) => (
                  <SelectItem key={profile.id} value={profile.id}>
                    {profile.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </EditableField>

          <EditableField label={t("title.rootFolder")}>
            <Select
              value={draft.rootFolderId}
              onValueChange={(value) =>
                setDraft((previous) => ({ ...previous, rootFolderId: value }))
              }
              disabled={busy}
            >
              <SelectTrigger className="h-9 w-full font-[var(--font-code)] text-sm">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={UNCHANGED_VALUE}>
                  {t("label.unchanged")}
                </SelectItem>
                {sortedRootFolders.map((rootFolder) => (
                  <SelectItem key={rootFolder.id} value={rootFolder.id}>
                    {rootFolder.isDefault
                      ? t("title.defaultRootFolder", {
                          path: folderLabel(rootFolder.path),
                        })
                      : folderLabel(rootFolder.path)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </EditableField>

          <EditableField label={t("search.addConfigMonitorType")}>
            <Select
              value={draft.monitorType}
              onValueChange={(value) =>
                setDraft((previous) => ({ ...previous, monitorType: value }))
              }
              disabled={busy}
            >
              <SelectTrigger className="h-9 w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={UNCHANGED_VALUE}>
                  {t("label.unchanged")}
                </SelectItem>
                {monitorOptions.map((option) => (
                  <SelectItem key={option.value} value={option.value}>
                    {option.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </EditableField>

          {!isMovieView ? (
            <EditableField label={t("search.addConfigSeasonFolder")}>
              <Select
                value={draft.useSeasonFolders}
                onValueChange={(value) =>
                  setDraft((previous) => ({
                    ...previous,
                    useSeasonFolders: value,
                  }))
                }
                disabled={busy}
              >
                <SelectTrigger className="h-9 w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={UNCHANGED_VALUE}>
                    {t("label.unchanged")}
                  </SelectItem>
                  <SelectItem value={ENABLED_VALUE}>
                    {t("search.seasonFolder.enabled")}
                  </SelectItem>
                  <SelectItem value={DISABLED_VALUE}>
                    {t("search.seasonFolder.disabled")}
                  </SelectItem>
                </SelectContent>
              </Select>
            </EditableField>
          ) : null}

          {isAnimeView ? (
            <EditableField label={t("settings.monitorSpecialsLabel")}>
              <Select
                value={draft.monitorSpecials}
                onValueChange={(value) =>
                  setDraft((previous) => ({
                    ...previous,
                    monitorSpecials: value,
                  }))
                }
                disabled={busy}
              >
                <SelectTrigger className="h-9 w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={UNCHANGED_VALUE}>
                    {t("label.unchanged")}
                  </SelectItem>
                  <SelectItem value={ENABLED_VALUE}>{t("label.enabled")}</SelectItem>
                  <SelectItem value={DISABLED_VALUE}>{t("label.disabled")}</SelectItem>
                </SelectContent>
              </Select>
            </EditableField>
          ) : null}

          {isAnimeView ? (
            <EditableField label={t("settings.interSeasonMoviesLabel")}>
              <Select
                value={draft.interSeasonMovies}
                onValueChange={(value) =>
                  setDraft((previous) => ({
                    ...previous,
                    interSeasonMovies: value,
                  }))
                }
                disabled={busy}
              >
                <SelectTrigger className="h-9 w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={UNCHANGED_VALUE}>
                    {t("label.unchanged")}
                  </SelectItem>
                  <SelectItem value={ENABLED_VALUE}>{t("label.enabled")}</SelectItem>
                  <SelectItem value={DISABLED_VALUE}>{t("label.disabled")}</SelectItem>
                </SelectContent>
              </Select>
            </EditableField>
          ) : null}

          {isAnimeView ? (
            <EditableField label={t("settings.fillerPolicyLabel")}>
              <Select
                value={draft.fillerPolicy}
                onValueChange={(value) =>
                  setDraft((previous) => ({ ...previous, fillerPolicy: value }))
                }
                disabled={busy}
              >
                <SelectTrigger className="h-9 w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={UNCHANGED_VALUE}>
                    {t("label.unchanged")}
                  </SelectItem>
                  <SelectItem value={INHERIT_VALUE}>
                    {t("title.inheritDefault")}
                  </SelectItem>
                  <SelectItem value="download_all">
                    {t("settings.fillerPolicyDownloadAll")}
                  </SelectItem>
                  <SelectItem value="skip_filler">
                    {t("settings.fillerPolicySkipFiller")}
                  </SelectItem>
                </SelectContent>
              </Select>
            </EditableField>
          ) : null}

          {isAnimeView ? (
            <EditableField label={t("settings.recapPolicyLabel")}>
              <Select
                value={draft.recapPolicy}
                onValueChange={(value) =>
                  setDraft((previous) => ({ ...previous, recapPolicy: value }))
                }
                disabled={busy}
              >
                <SelectTrigger className="h-9 w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={UNCHANGED_VALUE}>
                    {t("label.unchanged")}
                  </SelectItem>
                  <SelectItem value={INHERIT_VALUE}>
                    {t("title.inheritDefault")}
                  </SelectItem>
                  <SelectItem value="download_all">
                    {t("settings.recapPolicyDownloadAll")}
                  </SelectItem>
                  <SelectItem value="skip_recap">
                    {t("settings.recapPolicySkipRecap")}
                  </SelectItem>
                </SelectContent>
              </Select>
            </EditableField>
          ) : null}
        </div>

        <DialogFooter>
          <Button
            type="button"
            variant="outline"
            onClick={() => onOpenChange(false)}
            disabled={busy}
          >
            {t("label.cancel")}
          </Button>
          <Button
            type="button"
            variant="primary"
            onClick={handleSubmit}
            disabled={busy || !hasPendingChange}
          >
            {busy ? t("label.saving") : t("label.save")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function EditableField({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="space-y-2 rounded-lg border border-border/70 bg-muted/20 p-3">
      <p className="text-sm font-medium text-card-foreground">{label}</p>
      {children}
    </div>
  );
}
