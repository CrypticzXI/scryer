import * as React from "react";
import { useLocation } from "react-router-dom";
import { useTranslate } from "@/lib/context/translate-context";
import { Eye, EyeOff } from "lucide-react";
import type { OverviewTitleTarget, ViewId } from "@/components/root/types";
import { persistOverviewWindowScroll } from "@/lib/hooks/use-overview-window-scroll-restoration";
import type { TitleRecord } from "@/lib/types";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";
import { TitlePosterSlot } from "@/components/title-poster-slot";
import { titleOverviewOpenButtonId } from "@/lib/utils/dom-ids";
import { cn } from "@/lib/utils";
import {
  TitleCollectionEmptyState,
  TitleCollectionLoadingState,
} from "./title-table-shared";

type PosterGridProps = {
  titles: TitleRecord[];
  catalogInitialLoadComplete?: boolean;
  onOpenOverview: (
    targetView: ViewId,
    overviewTarget: OverviewTitleTarget,
  ) => void;
  selectedTitleId?: string | null;
  contextPanelId?: string;
  onSelectTitle?: (title: TitleRecord) => void;
  onDelete: (title: TitleRecord) => void;
  onAutoQueue: (title: TitleRecord) => void;
  isDeletingById: Record<string, boolean>;
  overviewTargetView: ViewId;
  showScanLibraryAction?: boolean;
  showConfigureRootsAction?: boolean;
  configureRootsReason?: "missing" | "invalid";
  configureRootsHref?: string;
  onScanLibrary?: () => Promise<void> | void;
  scanLibraryLoading?: boolean;
  scanLibraryDisabled?: boolean;
  scanLibraryNotice?: string | null;
};

export const PosterGrid = React.memo(function PosterGrid({
  titles,
  catalogInitialLoadComplete = true,
  onOpenOverview,
  selectedTitleId,
  contextPanelId,
  onSelectTitle,
  overviewTargetView,
  showScanLibraryAction = false,
  showConfigureRootsAction = false,
  configureRootsReason = "missing",
  configureRootsHref,
  onScanLibrary,
  scanLibraryLoading = false,
  scanLibraryDisabled = false,
  scanLibraryNotice,
}: PosterGridProps) {
  const t = useTranslate();

  if (!catalogInitialLoadComplete) {
    return <TitleCollectionLoadingState />;
  }

  if (titles.length === 0) {
    return (
      <TitleCollectionEmptyState
        t={t}
        showScanAction={showScanLibraryAction}
        showConfigureRootsAction={showConfigureRootsAction}
        configureRootsReason={configureRootsReason}
        configureRootsHref={configureRootsHref}
        scanLoading={scanLibraryLoading}
        scanDisabled={scanLibraryDisabled}
        scanNotice={scanLibraryNotice}
        onScan={onScanLibrary}
      />
    );
  }

  return (
    <div className="grid grid-cols-[repeat(auto-fill,minmax(150px,1fr))] gap-3.5">
      {titles.map((title) => (
        <PosterCard
          key={title.id}
          title={title}
          onOpenOverview={onOpenOverview}
          selected={selectedTitleId === title.id}
          contextPanelId={contextPanelId}
          onSelectTitle={onSelectTitle}
          overviewTargetView={overviewTargetView}
        />
      ))}
    </div>
  );
});

type PosterCardProps = {
  title: TitleRecord;
  onOpenOverview: (
    targetView: ViewId,
    overviewTarget: OverviewTitleTarget,
  ) => void;
  selected: boolean;
  contextPanelId?: string;
  onSelectTitle?: (title: TitleRecord) => void;
  overviewTargetView: ViewId;
};

const PosterCard = React.memo(function PosterCard({
  title,
  onOpenOverview,
  selected,
  contextPanelId,
  onSelectTitle,
  overviewTargetView,
}: PosterCardProps) {
  const location = useLocation();
  const t = useTranslate();
  const posterUrl = selectPosterVariantUrl(title.posterUrl, "w250");
  const posterClassName = "h-full w-full object-cover";
  const contextPanelControlsId =
    selected && onSelectTitle ? contextPanelId : undefined;
  const handleActivate = React.useCallback(() => {
    if (onSelectTitle) {
      persistOverviewWindowScroll(location.pathname);
      onSelectTitle(title);
      return;
    }
    persistOverviewWindowScroll(location.pathname);
    onOpenOverview(overviewTargetView, title);
  }, [
    location.pathname,
    onOpenOverview,
    onSelectTitle,
    overviewTargetView,
    title,
  ]);

  return (
    <div
      className={cn(
        "cv-auto-poster group",
      )}
    >
      <div
        className={cn(
          "overflow-hidden rounded-[12px] border bg-[var(--scry-card2)]",
          selected
            ? "border-[var(--scry-accent-ring)]"
            : "border-[var(--scry-border2)]",
        )}
      >
        <div className="relative">
          <button
            id={titleOverviewOpenButtonId(title.id)}
            type="button"
            onClick={handleActivate}
            aria-current={selected ? "true" : undefined}
            aria-controls={contextPanelControlsId}
            className="block w-full overflow-hidden rounded-[11px] bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            aria-label={title.name}
          >
            <div className="relative isolate aspect-[2/3] overflow-hidden rounded-[11px]">
              <TitlePosterSlot
                src={posterUrl}
                sourceSrc={title.posterSourceUrl}
                metadataFetchedAt={title.metadataFetchedAt}
                createdAt={title.createdAt}
                alt={t("media.posterAlt", { name: title.name })}
                className={posterClassName}
                placeholderClassName="flex h-full w-full items-center justify-center text-sm text-muted-foreground"
                emptyLabel={t("label.noArt")}
                loading="lazy"
                decoding="async"
              />

              <div
                className={cn(
                  "pointer-events-none absolute inset-0 z-10 overflow-hidden rounded-[calc(var(--radius)-1px)] opacity-0 transition-opacity duration-200 group-hover:opacity-100 group-focus-within:opacity-100",
                  selected && "opacity-100",
                )}
              >
                <div
                  aria-hidden="true"
                  className="absolute inset-0 rounded-[11px] border border-white/15 bg-gradient-to-t from-black/82 via-black/18 to-transparent"
                />
              </div>
              <div
                className={cn(
                  "pointer-events-none absolute inset-x-0 bottom-0 z-20 px-3 pb-3 pt-10 text-left opacity-0 transition-opacity duration-200 group-hover:opacity-100 group-focus-within:opacity-100",
                  selected && "opacity-100",
                )}
              >
                <p
                  className="line-clamp-2 text-[13px] font-semibold leading-tight tracking-normal text-white"
                  style={{
                    fontFamily:
                      "var(--font-space-grotesk), var(--font-inter), ui-sans-serif, system-ui, -apple-system, sans-serif",
                  }}
                >
                  {title.name}
                </p>
              </div>

              <div className="absolute left-[7px] top-[7px] z-20 flex h-[25px] w-[25px] items-center justify-center rounded-[7px] border border-white/10 bg-black/65 backdrop-blur-[4px]">
                {title.monitored ? (
                  <Eye className="h-3.5 w-3.5 text-emerald-400" />
                ) : (
                  <EyeOff className="h-3.5 w-3.5 text-[var(--scry-faint2)]" />
                )}
              </div>
            </div>
          </button>
        </div>
      </div>
    </div>
  );
});
