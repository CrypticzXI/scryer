import type { ReleaseQueueScope } from "./releases";

export type DownloadQueueState =
  | "queued"
  | "downloading"
  | "verifying"
  | "repairing"
  | "extracting"
  | "paused"
  | "completed"
  | "import_pending"
  | "failed";

export type ImportStatus =
  | "pending"
  | "running"
  | "processing"
  | "completed"
  | "failed"
  | "skipped";

export type ImportErrorCode =
  | "file_not_found"
  | "episode_not_found"
  | "episode_lookup_failed"
  | "io_failed"
  | "permission_denied"
  | "disk_full"
  | "unknown";

export type DownloadQueueDeleteStatus =
  | "queued"
  | "running"
  | "completed"
  | "failed";

export type TrackedDownloadState =
  | "downloading"
  | "import_pending"
  | "importing"
  | "imported"
  | "import_blocked"
  | "failed_pending"
  | "failed"
  | "ignored";

export type TrackedDownloadStatus = "ok" | "warning" | "error";

export type DownloadDisplayState =
  | "queued"
  | "downloading"
  | "paused"
  | "post_processing"
  | "completed"
  | "failed"
  | "importing"
  | "import_pending"
  | "import_blocked"
  | "import_failed"
  | "removing"
  | "remove_failed";

export type DownloadActivityFilter =
  | "all"
  | "downloading"
  | "queued"
  | "paused"
  | "post_processing";

export type DownloadImportFilter =
  | "all"
  | "importing"
  | "pending"
  | "blocked"
  | "failed";

export type DownloadHistoryFilter = "all" | "success" | "failed";
export type DownloadActivityStatus = Exclude<DownloadActivityFilter, "all">;
export type DownloadImportStatus = Exclude<DownloadImportFilter, "all">;
export type DownloadHistoryStatus = Exclude<DownloadHistoryFilter, "all">;
export type ActivitySortKey = "title" | "client" | "status" | "progress" | "size";
export type SortDirection = "asc" | "desc";
export type SortConfig = {
  key: ActivitySortKey;
  direction: SortDirection;
};

export type TitleMatchType =
  | "submission"
  | "client_parameter"
  | "title_parse"
  | "id_only"
  | "unmatched";

export type DownloadQueueItem = {
  id: string;
  titleId: string | null;
  episodeId: string | null;
  titleName: string;
  facet: string | null;
  isScryerOrigin: boolean;
  clientId: string;
  clientName: string;
  clientType: string;
  state: DownloadQueueState;
  displayState: DownloadDisplayState;
  progressPercent: number;
  sizeBytes: string | null;
  remainingSeconds: number | null;
  queuedAt: string | null;
  lastUpdatedAt: string | null;
  attentionRequired: boolean;
  attentionReason: string | null;
  downloadClientItemId: string;
  importStatus: ImportStatus | null;
  importErrorCode: ImportErrorCode | null;
  importErrorMessage: string | null;
  importedAt: string | null;
  deleteStatus: DownloadQueueDeleteStatus | null;
  deleteErrorMessage: string | null;
  trackedState: TrackedDownloadState | null;
  trackedStatus: TrackedDownloadStatus | null;
  trackedStatusMessages: string[];
  trackedMatchType: TitleMatchType | null;
  queueScope: ReleaseQueueScope | null;
};

export type DownloadHistoryPage = {
  items: DownloadQueueItem[];
  hasMore: boolean;
  totalCount: number;
  availableClients: DownloadClientFilterOption[];
};

export type DownloadImportPage = {
  items: DownloadQueueItem[];
  hasMore: boolean;
  totalCount: number;
};

export type DownloadClientFilterOption = {
  clientId: string;
  clientName: string;
  clientType: string;
};
