export type ViewId = "movies" | "series" | "anime" | "activity" | "calendar" | "wanted" | "history" | "settings" | "system";
export type SystemSection = "overview" | "jobs";
export type ActivitySection = "activity" | "import" | "history";
export type WantedSection = "wanted" | "cutoff" | "pending" | "history";
export type SettingsSection =
  | "profile"
  | "general"
  | "users"
  | "indexers"
  | "downloadClients"
  | "qualityProfiles"
  | "delayProfiles"
  | "acquisition"
  | "rules"
  | "plugins"
  | "notifications"
  | "post-processing"
  | "subtitles"
  | "recycleBin";
export type ContentSettingsSection =
  | "overview"
  | "import"
  | "settings"
  | "general"
  | "quality"
  | "renaming"
  | "routing";

export type OverviewTitleTarget = {
  id: string;
  slug?: string | null;
};

export type Translate = (
  key: string,
  values?: Record<string, string | number | boolean | null | undefined>,
) => string;

export type ActivityEvent = {
  id: string;
  kind: string;
  severity: string;
  channels: string[];
  eventType?: string;
  message: string;
  actorUserId?: string | null;
  titleId?: string | null;
  occurredAt?: string | null;
};

export type IndexerQueryStats = {
  indexerId: string;
  indexerName: string;
  queriesLast24H: number;
  successfulLast24H: number;
  failedLast24H: number;
  lastQueryAt: string | null;
  apiCurrent: number | null;
  apiMax: number | null;
  grabCurrent: number | null;
  grabMax: number | null;
};

export type SystemHealth = {
  serviceReady: boolean;
  dbPath: string;
  totalTitles: number;
  monitoredTitles: number;
  totalUsers: number;
  titlesMovie: number;
  titlesSeries: number;
  titlesAnime: number;
  titlesOther: number;
  recentEvents: number;
  recentEventPreview: string[];
  dbMigrationVersion: string | null;
  dbPendingMigrations: number;
  smgCertExpiresAt: string | null;
  smgCertDaysRemaining: number | null;
  indexerStats: IndexerQueryStats[];
};

export type SmgVersionCompatibilityNotice = {
  status: string;
  minimumVersion: string;
  yourVersion: string;
  message: string;
  upgradeDeadline: string | null;
};
