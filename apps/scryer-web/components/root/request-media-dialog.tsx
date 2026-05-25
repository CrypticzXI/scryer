import * as React from "react";
import { Loader2, Send } from "lucide-react";

import { TitlePoster } from "@/components/title-poster";
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
import { useTranslate } from "@/lib/context/translate-context";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import type { MetadataCatalogRequestOptions } from "@/lib/hooks/use-global-search";
import type { Facet, LibraryRecord } from "@/lib/types";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";

type RequestMediaDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  result: MetadataTvdbSearchItem;
  facet: Facet;
  requestableLibraries: LibraryRecord[];
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
  onRequest,
}: RequestMediaDialogProps) {
  const t = useTranslate();
  const [libraryId, setLibraryId] = React.useState("");
  const [isSubmitting, setIsSubmitting] = React.useState(false);

  React.useEffect(() => {
    if (!open) return;
    setLibraryId(
      requestableLibraries.find((library) => library.isDefault)?.id ||
        requestableLibraries[0]?.id ||
        "",
    );
    setIsSubmitting(false);
  }, [open, requestableLibraries]);

  const selectedLibrary = requestableLibraries.find((library) => library.id === libraryId) ?? null;
  const posterUrl = selectPosterVariantUrl(result.posterUrl, "w70");

  const handleSubmit = React.useCallback(async () => {
    const selectedLibraryId = selectedLibrary?.id.trim();
    if (!selectedLibraryId) return;

    setIsSubmitting(true);
    try {
      const accepted = await onRequest(result, facet, { libraryId: selectedLibraryId });
      if (accepted) {
        onOpenChange(false);
      }
    } finally {
      setIsSubmitting(false);
    }
  }, [facet, onOpenChange, onRequest, result, selectedLibrary]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent id="request-media-dialog" className="sm:max-w-md">
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

        <label className="space-y-1">
          <span className="block text-xs font-medium text-card-foreground">
            {t("search.addConfigLibrary")}
          </span>
          <Select
            value={selectedLibrary?.id || ""}
            onValueChange={setLibraryId}
            disabled={isSubmitting || requestableLibraries.length <= 1}
          >
            <SelectTrigger id="request-media-library" className="h-9 w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {requestableLibraries.map((library) => (
                <SelectItem key={library.id} value={library.id}>
                  {library.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </label>

        <DialogFooter>
          <Button
            id="request-media-cancel"
            type="button"
            variant="outline"
            onClick={() => onOpenChange(false)}
            disabled={isSubmitting}
          >
            {t("label.cancel")}
          </Button>
          <Button
            id="request-media-submit"
            type="button"
            onClick={() => void handleSubmit()}
            disabled={isSubmitting || !selectedLibrary}
            className="bg-emerald-600 text-foreground hover:bg-emerald-500"
          >
            {isSubmitting ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <Send className="h-4 w-4" />
            )}
            {isSubmitting ? t("search.requesting") : t("search.request")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
