import * as React from "react";
import { useLocation } from "react-router-dom";
import { useTranslate } from "@/lib/context/translate-context";
import type { OverviewTitleTarget, ViewId } from "@/components/root/types";
import { persistOverviewWindowScroll } from "@/lib/hooks/use-overview-window-scroll-restoration";
import type { TitleRecord } from "@/lib/types";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";
import { TitleCard } from "@/components/title-card";
import { titleOverviewOpenButtonId } from "@/lib/utils/dom-ids";
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
    <div className="cv-auto-poster">
      <TitleCard
        title={title.name}
        year={title.year ?? null}
        posterUrl={posterUrl}
        posterSourceUrl={title.posterSourceUrl}
        metadataFetchedAt={title.metadataFetchedAt}
        createdAt={title.createdAt}
        monitored={title.monitored}
        selected={selected}
        emptyLabel={t("label.noArt")}
        onOpen={handleActivate}
        interactiveProps={{
          id: titleOverviewOpenButtonId(title.id),
          "aria-current": selected ? "true" : undefined,
          "aria-controls": contextPanelControlsId,
        }}
      />
    </div>
  );
});
