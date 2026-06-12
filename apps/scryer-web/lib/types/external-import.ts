export type ExternalImportRootFolder = {
  source: string;
  path: string;
};

export type ExternalImportDownloadClient = {
  sources: string[];
  name: string;
  implementation: string;
  scryerClientType: string | null;
  host: string | null;
  port: string | null;
  useSsl: boolean;
  urlBase: string | null;
  username: string | null;
  apiKey: string | null;
  dedupKey: string;
  supported: boolean;
  requiresPasswordOverride: boolean;
};

export type ExternalImportIndexer = {
  sources: string[];
  name: string;
  implementation: string;
  scryerProviderType: string | null;
  baseUrl: string | null;
  apiKey: string | null;
  dedupKey: string;
  supported: boolean;
  childCount: number;
  childNames: string[];
  requiresApiKeyOverride: boolean;
  apiKeyHelpUrl: string | null;
};

export type ExternalImportPreview = {
  sonarrConnected: boolean;
  radarrConnected: boolean;
  prowlarrConnected: boolean;
  sonarrVersion: string | null;
  radarrVersion: string | null;
  prowlarrVersion: string | null;
  sonarrError: string | null;
  radarrError: string | null;
  prowlarrError: string | null;
  rootFolders: ExternalImportRootFolder[];
  downloadClients: ExternalImportDownloadClient[];
  indexers: ExternalImportIndexer[];
};

export type ExternalImportResult = {
  mediaPathsSaved: boolean;
  downloadClientsCreated: number;
  indexersCreated: number;
  pluginsInstalled: string[];
  errors: string[];
};

export type ExternalImportConnection = {
  baseUrl: string;
  apiKey: string;
};

export type ExternalImportMonitorWarmupStatus =
  | "queued"
  | "running"
  | "completed"
  | "canceled"
  | "failed";

export type ExternalImportMonitorWarmupPhase =
  | "loading_movies"
  | "loading_series"
  | "loading_episodes"
  | "building_snapshot"
  | "ready";

export type ExternalImportMonitorWarmupPhaseProgress = {
  total: number;
  completed: number;
  failed: number;
};

export type ExternalImportMonitorWarmupProgress = {
  sessionId: string;
  status: ExternalImportMonitorWarmupStatus;
  phase: ExternalImportMonitorWarmupPhase;
  startedAt: string;
  updatedAt: string;
  overallTotalKnown: boolean;
  overallProgress: ExternalImportMonitorWarmupPhaseProgress;
  moviesTotalKnown: boolean;
  moviesProgress: ExternalImportMonitorWarmupPhaseProgress;
  seriesTotalKnown: boolean;
  seriesProgress: ExternalImportMonitorWarmupPhaseProgress;
  episodeFetchTotalKnown: boolean;
  episodeFetchExpectedTotal: number | null;
  episodeFetchExpectedMonitoredTotal: number | null;
  episodeFetchProgress: ExternalImportMonitorWarmupPhaseProgress;
  snapshotBuildTotalKnown: boolean;
  snapshotBuildProgress: ExternalImportMonitorWarmupPhaseProgress;
  matchedMovieCount: number;
  matchedSeriesCount: number;
  unmatchedMovieCount: number;
  unmatchedSeriesCount: number;
  ambiguousMovieCount: number;
  ambiguousSeriesCount: number;
  errorMessage: string | null;
};
