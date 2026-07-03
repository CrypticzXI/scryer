import * as React from "react";
import { Loader2, Search } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { SearchResultBuckets } from "@/components/common/release-search-results";
import { TitleSearchDownloadClientNotice } from "@/components/common/title-search-download-client-notice";
import { useTranslate } from "@/lib/context/translate-context";
import type { Release } from "@/lib/types";
import { releaseSupportsAdditionalFileQueue } from "@/lib/utils/release-queue-scope";
import { selectorId } from "@/lib/utils/dom-ids";
import type {
  CollectionEpisode,
  EpisodeMediaFile,
  TitleCollection,
  TitleReleaseBlocklistEntry,
} from "@/components/containers/series-overview-container";
import type { EpisodePanelTab } from "./episode-panel-reducer";
import type { ExternalSubtitleRecord } from "@/lib/types/subtitles";
import { blocklistEntryMatchesEpisode } from "./helpers";
import { EpisodeDetailsPanel } from "./episode-details-panel";
import { EpisodeBlocklistPanel } from "./episode-blocklist-panel";
import { EMPTY_BLOCKLIST_ENTRIES } from "./season-section-utils";

export type EpisodePanelContentProps = {
  activeTab: EpisodePanelTab;
  canClearBlocklistEntries: boolean;
  collection: TitleCollection;
  clearingReleaseBlocklistEntryId?: string | null;
  episode: CollectionEpisode;
  episodeFiles: EpisodeMediaFile[];
  episodeLoading: boolean;
  episodeResults: Release[];
  facet: string;
  onClearReleaseBlocklistEntry?: (entryId: string) => Promise<void> | void;
  onDeleteFile?: (fileId: string) => void;
  onQueueFromEpisodeSearch?: (episode: CollectionEpisode, release: Release) => Promise<void> | void;
  onQueueAdditionalFromEpisodeSearch?: (episode: CollectionEpisode, release: Release) => Promise<void> | void;
  onRefreshSubtitles?: () => Promise<void> | void;
  onRunEpisodeSearch?: (episode: CollectionEpisode) => void;
  onTabChange: (tab: EpisodePanelTab | "history") => void;
  showHistoryTab: boolean;
  showSearchTab: boolean;
  releaseBlocklistEntries: TitleReleaseBlocklistEntry[];
  searchBlocked: boolean;
  subtitleDownloads: ExternalSubtitleRecord[];
};

export const EpisodePanelContent = React.memo(function EpisodePanelContent({
  activeTab,
  canClearBlocklistEntries,
  collection,
  clearingReleaseBlocklistEntryId,
  episode,
  episodeFiles,
  episodeLoading,
  episodeResults,
  facet,
  onClearReleaseBlocklistEntry,
  onDeleteFile,
  onQueueFromEpisodeSearch,
  onQueueAdditionalFromEpisodeSearch,
  onRefreshSubtitles,
  onRunEpisodeSearch,
  onTabChange,
  showHistoryTab,
  showSearchTab,
  releaseBlocklistEntries,
  searchBlocked,
  subtitleDownloads,
}: EpisodePanelContentProps) {
  const t = useTranslate();
  const filteredBlocklistEntries = React.useMemo(() => {
    if (activeTab !== "blocklist") {
      return EMPTY_BLOCKLIST_ENTRIES;
    }

    return releaseBlocklistEntries.filter((entry) =>
      blocklistEntryMatchesEpisode(entry, episode, collection),
    );
  }, [activeTab, collection, episode, releaseBlocklistEntries]);

  return (
    <Tabs
      value={activeTab}
      onValueChange={(value) => onTabChange(value as EpisodePanelTab | "history")}
    >
      <TabsList className="flex w-full flex-nowrap overflow-x-auto">
        <TabsTrigger
          id={selectorId("series-overview-episode-tab", episode.id, "details")}
          value="details"
          className="shrink-0"
        >{t("episode.details")}</TabsTrigger>
        {showSearchTab ? (
          <TabsTrigger
            id={selectorId("series-overview-episode-tab", episode.id, "search")}
            value="search"
            className="shrink-0"
          >{t("episode.search")}</TabsTrigger>
        ) : null}
        {showHistoryTab ? (
          <TabsTrigger
            id={selectorId("series-overview-episode-tab", episode.id, "history")}
            value="history"
            className="shrink-0"
          >{t("history.title")}</TabsTrigger>
        ) : null}
        <TabsTrigger
          id={selectorId("series-overview-episode-tab", episode.id, "blocklist")}
          value="blocklist"
          className="shrink-0"
        >{t("episode.blocklist")}</TabsTrigger>
      </TabsList>
      <TabsContent value="details">
        <EpisodeDetailsPanel
          episode={episode}
          mediaFiles={episodeFiles}
          subtitleDownloads={subtitleDownloads}
          onRefreshSubtitles={onRefreshSubtitles}
          onDeleteFile={onDeleteFile}
        />
      </TabsContent>
      {showSearchTab ? (
        <TabsContent value="search">
        {searchBlocked ? (
          <TitleSearchDownloadClientNotice />
        ) : (
          <div className="mb-2 flex items-center justify-end">
            <Button
              id={selectorId("series-overview-episode-search-refresh", episode.id)}
              type="button"
              variant="ghost"
              size="sm"
              onClick={() => onRunEpisodeSearch?.(episode)}
              disabled={episodeLoading}
              aria-label={t("label.search")}
            >
              <Search className="h-4 w-4" />
              <span className="ml-1">
                {episodeLoading ? t("label.searching") : t("label.refresh")}
              </span>
            </Button>
          </div>
        )}
        {searchBlocked ? null : episodeLoading ? (
          <div className="flex flex-col items-center justify-center gap-4 py-16">
            <Loader2 className="h-10 w-10 animate-spin text-[var(--scry-accent-text)]" />
            <p className="text-lg text-muted-foreground">{t("label.searching")}</p>
          </div>
        ) : episodeResults.length === 0 ? (
          <p className="text-sm text-muted-foreground">{t("nzb.noResultsYet")}</p>
        ) : (
          <SearchResultBuckets
            results={episodeResults}
            onQueue={(release) => onQueueFromEpisodeSearch?.(episode, release)}
            onQueueAdditional={(release) => onQueueAdditionalFromEpisodeSearch?.(episode, release)}
            canQueueAdditional={(release) =>
              releaseSupportsAdditionalFileQueue(release, facet)
            }
            requireCandidateToken
          />
        )}
        </TabsContent>
      ) : null}
      <TabsContent value="blocklist">
        <EpisodeBlocklistPanel
          entries={filteredBlocklistEntries}
          canClear={canClearBlocklistEntries}
          clearingEntryId={clearingReleaseBlocklistEntryId}
          onClear={onClearReleaseBlocklistEntry}
        />
      </TabsContent>
    </Tabs>
  );
});
