import * as React from "react";
import { Loader2, Send } from "lucide-react";

import { CatalogActionDialogSummary } from "@/components/root/catalog-action-dialog-summary";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogFooter } from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useTranslate } from "@/lib/context/translate-context";
import { defaultMonitorTypeForFacet } from "@/lib/facets/helpers";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import type {
  CatalogQualityProfileOption,
  MetadataCatalogMonitorType,
  MetadataCatalogRequestOptions,
} from "@/lib/hooks/use-global-search";
import type { Facet, LibraryRecord } from "@/lib/types";
import {
  mediaRequestMonitorOptionId,
  mediaRequestProfileOptionId,
  selectorId,
} from "@/lib/utils/dom-ids";

type RequestMediaDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  result: MetadataTvdbSearchItem;
  facet: Facet;
  requestableLibraries: LibraryRecord[];
  qualityProfileOptions: CatalogQualityProfileOption[];
  onRequest: (
    result: MetadataTvdbSearchItem,
    facet: Facet,
    options: MetadataCatalogRequestOptions,
  ) => Promise<boolean>;
};

export function RequestMediaDialog({
  open,
  onOpenChange,
  result,
  facet,
  requestableLibraries,
  qualityProfileOptions,
  onRequest,
}: RequestMediaDialogProps) {
  const t = useTranslate();
  const [libraryId, setLibraryId] = React.useState("");
  const [qualityProfileId, setQualityProfileId] = React.useState("");
  const [monitorType, setMonitorType] = React.useState<MetadataCatalogMonitorType>(
    () => defaultMonitorTypeForFacet(facet),
  );
  const [isSubmitting, setIsSubmitting] = React.useState(false);

  React.useEffect(() => {
    if (!open) return;
    setLibraryId(
      requestableLibraries.find((library) => library.isDefault)?.id ||
        requestableLibraries[0]?.id ||
        "",
    );
    setQualityProfileId("");
    setMonitorType(defaultMonitorTypeForFacet(facet));
    setIsSubmitting(false);
  }, [facet, open, requestableLibraries]);

  const selectedLibrary = requestableLibraries.find((library) => library.id === libraryId) ?? null;
  const canRequestMonitorType = facet !== "movie";
  const monitorOptions: Array<{ value: MetadataCatalogMonitorType; label: string }> = [
    { value: "futureEpisodes", label: t("search.monitorType.futureEpisodes") },
    {
      value: "missingAndFutureEpisodes",
      label: t("search.monitorType.missingAndFutureEpisodes"),
    },
    { value: "allEpisodes", label: t("search.monitorType.allEpisodes") },
    { value: "none", label: t("search.monitorType.none") },
  ];
  const requestProfileOptions = React.useMemo(() => {
    const requestProfileIds = selectedLibrary?.requestQualityProfileIds?.length
      ? selectedLibrary.requestQualityProfileIds
      : selectedLibrary?.requestQualityProfileDefaultId
        ? [selectedLibrary.requestQualityProfileDefaultId]
        : [];
    return requestProfileIds.map((profileId) => {
      const profile = qualityProfileOptions.find((option) => option.id === profileId);
      return {
        id: profileId,
        name: profile?.name ?? profileId,
      };
    });
  }, [qualityProfileOptions, selectedLibrary]);
  React.useEffect(() => {
    if (!open || !selectedLibrary) return;
    const defaultProfileId =
      selectedLibrary.requestQualityProfileDefaultId ||
      requestProfileOptions[0]?.id ||
      "";
    setQualityProfileId((current) =>
      current && requestProfileOptions.some((profile) => profile.id === current)
        ? current
        : defaultProfileId,
    );
  }, [open, requestProfileOptions, selectedLibrary]);

  const handleSubmit = React.useCallback(async () => {
    const selectedLibraryId = selectedLibrary?.id.trim();
    const selectedQualityProfileId = qualityProfileId.trim();
    if (!selectedLibraryId || !selectedQualityProfileId) return;

    setIsSubmitting(true);
    try {
      const accepted = await onRequest(result, facet, {
        libraryId: selectedLibraryId,
        requestedQualityProfileId: selectedQualityProfileId,
        requestedMonitorType: canRequestMonitorType ? monitorType : undefined,
      });
      if (accepted) {
        onOpenChange(false);
      }
    } finally {
      setIsSubmitting(false);
    }
  }, [
    canRequestMonitorType,
    facet,
    monitorType,
    onOpenChange,
    onRequest,
    qualityProfileId,
    result,
    selectedLibrary,
  ]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        id="request-media-dialog"
        className="max-h-[90vh] gap-0 overflow-y-auto p-0 sm:max-w-5xl"
      >
        <CatalogActionDialogSummary result={result} facet={facet} mode="request" />

        <div className="space-y-6 p-5 sm:p-7">
          <div className="grid gap-4 sm:grid-cols-2">
            <label className="space-y-1 sm:col-span-2">
              <span className="block text-xs font-medium text-card-foreground">
                {t("search.addConfigLibrary")}
              </span>
              <Select
                value={selectedLibrary?.id || ""}
                onValueChange={setLibraryId}
                disabled={isSubmitting || requestableLibraries.length <= 1}
              >
                <SelectTrigger id="request-media-library" className="h-12 w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {requestableLibraries.map((library) => (
                    <SelectItem
                      id={selectorId("request-media-library-option", library.id)}
                      key={library.id}
                      value={library.id}
                    >
                      {library.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </label>

            <label className="space-y-1">
              <span className="block text-xs font-medium text-card-foreground">
                {t("requests.requestedQualityProfile")}
              </span>
              <Select
                value={qualityProfileId}
                onValueChange={setQualityProfileId}
                disabled={isSubmitting || requestProfileOptions.length <= 1}
              >
                <SelectTrigger
                  id="request-media-quality-profile"
                  className="h-12 w-full"
                >
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {requestProfileOptions.map((profile) => (
                    <SelectItem
                      id={mediaRequestProfileOptionId("request", profile.id)}
                      key={profile.id}
                      value={profile.id}
                    >
                      {profile.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </label>

            {canRequestMonitorType ? (
              <label className="space-y-1">
                <span className="block text-xs font-medium text-card-foreground">
                  {t("requests.requestedMonitorType")}
                </span>
                <Select
                  value={monitorType}
                  onValueChange={(value) =>
                    setMonitorType(value as MetadataCatalogMonitorType)
                  }
                  disabled={isSubmitting}
                >
                  <SelectTrigger
                    id="request-media-monitor-type"
                    className="h-12 w-full"
                  >
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {monitorOptions.map((option) => (
                      <SelectItem
                        id={mediaRequestMonitorOptionId("request", option.value)}
                        key={option.value}
                        value={option.value}
                      >
                        {option.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </label>
            ) : null}
          </div>

        <DialogFooter className="items-stretch gap-3 sm:items-center">
          <Button
            id="request-media-cancel"
            type="button"
            variant="outline"
            onClick={() => onOpenChange(false)}
            disabled={isSubmitting}
            className="h-12 px-8"
          >
            {t("label.cancel")}
          </Button>
          <Button
            id="request-media-submit"
            type="button"
            onClick={() => void handleSubmit()}
            disabled={
              isSubmitting ||
              !selectedLibrary ||
              !qualityProfileId ||
              (canRequestMonitorType && !monitorType)
            }
            className="h-12 gap-2 bg-primary px-8 text-primary-foreground hover:bg-primary/90"
          >
            {isSubmitting ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <Send className="h-4 w-4" />
            )}
            {isSubmitting ? t("search.requesting") : t("search.request")}
          </Button>
        </DialogFooter>
        </div>
      </DialogContent>
    </Dialog>
  );
}
