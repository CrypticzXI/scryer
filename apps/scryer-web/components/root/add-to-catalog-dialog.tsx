import * as React from "react";
import { Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Checkbox } from "@/components/ui/checkbox";
import { useTranslate } from "@/lib/context/translate-context";
import { defaultMonitorTypeForFacet } from "@/lib/facets/helpers";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";
import { TitlePoster } from "@/components/title-poster";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import type { Facet } from "@/lib/types";
import type {
  CatalogQualityProfileOption,
  MetadataCatalogAddOptions,
  MetadataCatalogMonitorType,
} from "@/lib/hooks/use-global-search";
import type { LibraryRecord, RootFolderOption } from "@/lib/types/titles";

type AddToCatalogDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  result: MetadataTvdbSearchItem;
  facet: Facet;
  catalogQualityProfileOptions: CatalogQualityProfileOption[];
  catalogConfigLoading: boolean;
  defaultQualityProfileId: string;
  manageableLibraries: LibraryRecord[];
  rootFolderOptions: RootFolderOption[];
  onAdd: (
    result: MetadataTvdbSearchItem,
    facet: Facet,
    options: MetadataCatalogAddOptions,
  ) => Promise<string | null>;
};

/** Sentinel used by callers when the dialog is closed so they don't need to pass null. */
export const EMPTY_SEARCH_RESULT: MetadataTvdbSearchItem = {
  tvdbId: "",
  name: "",
  imdbId: null,
  slug: null,
  type: null,
  year: null,
  status: null,
  overview: null,
  popularity: null,
  posterUrl: null,
  language: null,
  runtimeMinutes: null,
  sortTitle: null,
};

function buildDefaultDraft(
  facet: Facet,
  defaultQualityProfileId: string,
  defaultLibraryId?: string,
  defaultRootFolderId?: string,
): MetadataCatalogAddOptions {
  return {
    libraryId: defaultLibraryId,
    qualityProfileId: defaultQualityProfileId,
    rootFolderId: defaultRootFolderId,
    seasonFolder: facet !== "movie",
    monitorType: defaultMonitorTypeForFacet(facet),
    ...(facet === "movie" ? { minAvailability: "announced" } : {}),
    ...(facet === "anime"
      ? {
          monitorSpecials: false,
          interSeasonMovies: true,
        }
      : {}),
  };
}

function defaultLibrary(libraries: LibraryRecord[]): LibraryRecord | null {
  return libraries.find((library) => library.isDefault) || libraries[0] || null;
}

function defaultRootFolderId(
  rootFolders: Array<{ id?: string; isDefault: boolean }>,
): string | undefined {
  return (
    rootFolders.find((rootFolder) => rootFolder.isDefault && rootFolder.id)?.id ||
    rootFolders.find((rootFolder) => rootFolder.id)?.id
  );
}

export function AddToCatalogDialog({
  open,
  onOpenChange,
  result,
  facet,
  catalogQualityProfileOptions,
  catalogConfigLoading,
  defaultQualityProfileId,
  manageableLibraries,
  rootFolderOptions,
  onAdd,
}: AddToCatalogDialogProps) {
  const t = useTranslate();
  const libraries = manageableLibraries;
  const fallbackRootFolders = rootFolderOptions;
  const [draft, setDraft] = React.useState<MetadataCatalogAddOptions>(() =>
    buildDefaultDraft(
      facet,
      defaultQualityProfileId,
      defaultLibrary(libraries)?.id,
      defaultRootFolderId(defaultLibrary(libraries)?.roots ?? fallbackRootFolders),
    ),
  );
  const [isSubmitting, setIsSubmitting] = React.useState(false);

  // Reset draft when dialog opens
  React.useEffect(() => {
    if (!open) return;
    const nextDefaultLibrary = defaultLibrary(libraries);
    setDraft(
      buildDefaultDraft(
        facet,
        defaultQualityProfileId,
        nextDefaultLibrary?.id,
        defaultRootFolderId(nextDefaultLibrary?.roots ?? fallbackRootFolders),
      ),
    );
    setIsSubmitting(false);
  }, [open, facet, defaultQualityProfileId, libraries, fallbackRootFolders]);

  const qualityProfileValue =
    draft.qualityProfileId || defaultQualityProfileId;
  const selectedLibrary =
    libraries.find((library) => library.id === draft.libraryId) ||
    defaultLibrary(libraries) ||
    null;
  const selectedRootFolders = selectedLibrary?.roots ?? fallbackRootFolders;
  const selectableRootFolders = selectedRootFolders.flatMap((rootFolder) => {
    const id = rootFolder.id?.trim();
    return id ? [{ ...rootFolder, id }] : [];
  });
  const draftRootFolderId = draft.rootFolderId?.trim();
  const effectiveRootFolderId =
    draftRootFolderId &&
    selectableRootFolders.some((rootFolder) => rootFolder.id === draftRootFolderId)
      ? draftRootFolderId
      : defaultRootFolderId(selectableRootFolders) || "";
  const libraryRequired = libraries.length > 0;
  const hasCatalogDestination =
    libraries.length > 0 || selectableRootFolders.length > 0;
  const qualityProfileSelectionDisabled =
    isSubmitting || catalogConfigLoading || catalogQualityProfileOptions.length === 0;

  const handleSubmit = React.useCallback(async () => {
    const libraryId = selectedLibrary?.id?.trim();
    if (!hasCatalogDestination || (libraryRequired && !libraryId)) return;

    setIsSubmitting(true);
    try {
      const qpId = (draft.qualityProfileId || defaultQualityProfileId).trim();
      if (!qpId) return;
      const titleId = await onAdd(result, facet, {
        ...draft,
        libraryId,
        qualityProfileId: qpId,
        rootFolderId: effectiveRootFolderId || undefined,
      });
      if (titleId) {
        onOpenChange(false);
      }
    } finally {
      setIsSubmitting(false);
    }
  }, [
    draft,
    defaultQualityProfileId,
    facet,
    hasCatalogDestination,
    libraryRequired,
    onAdd,
    onOpenChange,
    result,
    effectiveRootFolderId,
    selectedLibrary,
  ]);

  const update = React.useCallback(
    (patch: Partial<MetadataCatalogAddOptions>) => {
      setDraft((prev) => ({ ...prev, ...patch }));
    },
    [],
  );

  const monitorOptions: Array<{ value: MetadataCatalogMonitorType; label: string }> =
    facet === "movie"
      ? [
          { value: "monitored", label: t("search.monitorType.monitored") },
          { value: "unmonitored", label: t("search.monitorType.unmonitored") },
        ]
      : [
          { value: "futureEpisodes", label: t("search.monitorType.futureEpisodes") },
          {
            value: "missingAndFutureEpisodes",
            label: t("search.monitorType.missingAndFutureEpisodes"),
          },
          { value: "allEpisodes", label: t("search.monitorType.allEpisodes") },
          { value: "none", label: t("search.monitorType.none") },
        ];

  const posterUrl = selectPosterVariantUrl(result.posterUrl, "w70");

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent id="add-to-catalog-dialog" className="sm:max-w-md">
        <DialogHeader>
          <div className="flex gap-3">
            <div className="h-20 w-14 flex-none overflow-hidden rounded-md border border-border bg-muted">
              {posterUrl ? (
                <TitlePoster
                  src={posterUrl}
                  alt={t("media.posterAlt", { name: result.name })}
                  className="h-full w-full object-cover"
                />
              ) : (
                <div className="flex h-full w-full items-center justify-center text-xs text-muted-foreground">
                  {t("label.noArt")}
                </div>
              )}
            </div>
            <div className="min-w-0">
              <DialogTitle className="text-base">{result.name}</DialogTitle>
              <DialogDescription>
                {result.year ? result.year : t("label.yearUnknown")}
              </DialogDescription>
            </div>
          </div>
          {result.overview ? (
            <p className="mt-2 text-xs text-muted-foreground line-clamp-3">
              {result.overview}
            </p>
          ) : null}
        </DialogHeader>

        <div className="grid gap-3 sm:grid-cols-2">
          {libraries.length >= 1 ? (
            <label className="space-y-1 sm:col-span-2">
              <span className="block text-xs font-medium text-card-foreground">
                {t("search.addConfigLibrary")}
              </span>
              <Select
                value={selectedLibrary?.id || ""}
                onValueChange={(v) => update({ libraryId: v, rootFolderId: undefined })}
                disabled={isSubmitting || libraries.length === 1}
              >
                <SelectTrigger id="add-to-catalog-library" className="h-9 w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {libraries.map((library) => (
                    <SelectItem key={library.id} value={library.id}>
                      {library.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </label>
          ) : null}

          {(
            <label className="space-y-1">
              <span className="block text-xs font-medium text-card-foreground">
                {t("search.addConfigQualityProfile")}
              </span>
              <Select
                value={catalogQualityProfileOptions.length > 0 ? qualityProfileValue : ""}
                onValueChange={(v) => update({ qualityProfileId: v })}
                disabled={qualityProfileSelectionDisabled}
              >
                <SelectTrigger
                  id="add-to-catalog-quality-profile"
                  className="h-9 w-full"
                  aria-busy={catalogConfigLoading}
                >
                  <SelectValue placeholder={catalogConfigLoading ? t("label.loading") : undefined} />
                </SelectTrigger>
                <SelectContent>
                  {catalogQualityProfileOptions.length === 0 ? (
                    <SelectItem value="__none" disabled>
                      {t("search.addConfigNoQualityProfiles")}
                    </SelectItem>
                  ) : (
                    catalogQualityProfileOptions.map((profile) => (
                      <SelectItem key={profile.id} value={profile.id}>
                        {profile.name}
                      </SelectItem>
                    ))
                  )}
                </SelectContent>
              </Select>
            </label>
          )}

          {/* Root Folder */}
          {selectableRootFolders.length >= 1 ? (
            <label className="space-y-1">
              <span className="block text-xs font-medium text-card-foreground">
                {t("search.addConfigRootFolder")}
              </span>
              <Select
                value={effectiveRootFolderId}
                onValueChange={(v) => update({ rootFolderId: v })}
                disabled={isSubmitting}
              >
                <SelectTrigger id="add-to-catalog-root-folder" className="h-9 w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {selectableRootFolders.map((rf) => (
                    <SelectItem key={rf.id} value={rf.id}>
                      {rf.path}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </label>
          ) : null}

          {/* Season Folder — series + anime */}
          {facet !== "movie" ? (
            <label className="space-y-1">
              <span className="block text-xs font-medium text-card-foreground">
                {t("search.addConfigSeasonFolder")}
              </span>
              <Select
                value={draft.seasonFolder ? "enabled" : "disabled"}
                onValueChange={(v) => update({ seasonFolder: v === "enabled" })}
                disabled={isSubmitting}
              >
                <SelectTrigger id="add-to-catalog-season-folder" className="h-9 w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="enabled">{t("search.seasonFolder.enabled")}</SelectItem>
                  <SelectItem value="disabled">{t("search.seasonFolder.disabled")}</SelectItem>
                </SelectContent>
              </Select>
            </label>
          ) : null}

          {/* Monitored checkbox — movie only */}
          {facet === "movie" ? (
            <label className="flex items-center gap-2 sm:col-span-2">
              <Checkbox
                id="add-to-catalog-monitored"
                checked={draft.monitorType === "monitored"}
                onCheckedChange={(v) =>
                  update({ monitorType: v === true ? "monitored" : "unmonitored" })
                }
                disabled={isSubmitting}
              />
              <span className="text-sm text-card-foreground">
                {t("title.monitored")}
              </span>
            </label>
          ) : (
            /* Monitor Type — series + anime */
            <label className="space-y-1">
              <span className="block text-xs font-medium text-card-foreground">
                {t("search.addConfigMonitorType")}
              </span>
              <Select
                value={draft.monitorType}
                onValueChange={(v) =>
                  update({ monitorType: v as MetadataCatalogMonitorType })
                }
                disabled={isSubmitting}
              >
                <SelectTrigger id="add-to-catalog-monitor-type" className="h-9 w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {monitorOptions.map((option) => (
                    <SelectItem key={option.value} value={option.value}>
                      {option.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </label>
          )}
        </div>

        {catalogConfigLoading ? (
          <div
            id="add-to-catalog-config-loading"
            className="flex items-center gap-2 rounded-md border border-dashed border-border/80 bg-muted/30 px-3 py-2 text-sm text-muted-foreground"
          >
            <Loader2 className="h-4 w-4 animate-spin text-primary" />
            <span>{t("label.loading")}</span>
          </div>
        ) : null}

        <DialogFooter>
          <Button
            id="add-to-catalog-cancel"
            type="button"
            variant="outline"
            onClick={() => onOpenChange(false)}
            disabled={isSubmitting}
          >
            {t("label.cancel")}
          </Button>
          <Button
            id="add-to-catalog-submit"
            type="button"
            onClick={() => void handleSubmit()}
            disabled={
              isSubmitting ||
              catalogConfigLoading ||
              !qualityProfileValue ||
              !hasCatalogDestination ||
              (libraryRequired && !selectedLibrary)
            }
            className="bg-primary text-primary-foreground hover:bg-primary/90"
          >
            {isSubmitting ? t("search.adding") : t("title.addToCatalog")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
