import { RefreshCw } from "lucide-react";

import { LibraryMultiSelect } from "@/components/common/library-multi-select";
import { TitlePoster } from "@/components/title-poster";
import { Button } from "@/components/ui/button";
import { useTranslate } from "@/lib/context/translate-context";
import type { LibraryRecord, MediaRequestRecord } from "@/lib/types";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";
import { cn } from "@/lib/utils";

type RequestsViewProps = {
  libraries: LibraryRecord[];
  selectedLibraryIds: string[];
  onSelectedLibraryIdsChange: (libraryIds: string[]) => void;
  requests: MediaRequestRecord[];
  loading: boolean;
  onRefresh: () => void;
};

function requestExternalIdLabel(request: MediaRequestRecord): string {
  return request.externalIds
    .map((externalId) => `${externalId.source.toUpperCase()} ${externalId.value}`)
    .join(" / ");
}

function requesterLabel(request: MediaRequestRecord): string {
  return request.requesters
    .map((requester) => requester.username)
    .filter(Boolean)
    .join(", ");
}

export function RequestsView({
  libraries,
  selectedLibraryIds,
  onSelectedLibraryIdsChange,
  requests,
  loading,
  onRefresh,
}: RequestsViewProps) {
  const t = useTranslate();

  return (
    <section className="flex min-h-0 flex-1 flex-col gap-4">
      <div className="flex flex-wrap items-center gap-2">
        <LibraryMultiSelect
          libraries={libraries}
          selectedLibraryIds={selectedLibraryIds}
          onSelectedLibraryIdsChange={onSelectedLibraryIdsChange}
          triggerClassName="h-11 min-w-56"
        />
        <Button
          type="button"
          variant="outline"
          className="h-11"
          onClick={onRefresh}
          disabled={loading}
        >
          <RefreshCw className={cn("h-4 w-4", loading && "animate-spin")} />
        </Button>
      </div>

      <div className="grid gap-3">
        {requests.length === 0 && !loading ? (
          <div className="rounded-lg border border-dashed border-border bg-card/40 px-4 py-8 text-center text-sm text-muted-foreground">
            {t("requests.empty")}
          </div>
        ) : null}

        {requests.map((request) => {
          const posterUrl = selectPosterVariantUrl(request.posterUrl, "w70");
          const requesters = requesterLabel(request);
          const externalIds = requestExternalIdLabel(request);
          return (
            <article
              key={request.id}
              className="rounded-lg border border-border bg-card/60 p-3"
            >
              <div className="flex gap-3">
                <div className="h-24 w-16 flex-none overflow-hidden rounded-md border border-border bg-muted">
                  {posterUrl ? (
                    <TitlePoster
                      src={posterUrl}
                      alt={t("media.posterAlt", { name: request.title })}
                      className="h-full w-full object-cover"
                      loading="lazy"
                    />
                  ) : (
                    <div className="flex h-full w-full items-center justify-center text-xs text-muted-foreground">
                      {t("label.noArt")}
                    </div>
                  )}
                </div>
                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-start justify-between gap-2">
                    <div className="min-w-0">
                      <h2 className="truncate text-base font-semibold text-foreground">
                        {request.title}
                      </h2>
                      <p className="text-xs text-muted-foreground">
                        {request.year ?? t("label.yearUnknown")}
                        {externalIds ? ` • ${externalIds}` : ""}
                      </p>
                    </div>
                    <span className="rounded-md border border-border bg-background px-2 py-1 text-xs font-medium uppercase text-muted-foreground">
                      {request.status}
                    </span>
                  </div>
                  {request.overview ? (
                    <p className="mt-2 line-clamp-2 text-sm text-muted-foreground">
                      {request.overview}
                    </p>
                  ) : null}
                  <div className="mt-3 flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
                    <span>{t("requests.requesters")}: {requesters || t("label.unknown")}</span>
                    <span>{t("requests.updated")}: {new Date(request.updatedAt).toLocaleString()}</span>
                  </div>
                </div>
              </div>
            </article>
          );
        })}
      </div>
    </section>
  );
}
