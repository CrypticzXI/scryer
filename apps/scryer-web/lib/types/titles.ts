export type Facet = "movie" | "series" | "anime";

export type ExternalId = {
  source: string;
  value: string;
};

export type TitleRecord = {
  id: string;
  name: string;
  facet: Facet;
  monitored: boolean;
  tags: string[];
  createdAt?: string | null;
  year?: number | null;
  overview?: string | null;
  sortTitle?: string | null;
  slug?: string | null;
  imdbId?: string | null;
  externalIds: ExternalId[];
  qualityTier?: string | null;
  currentQualityTier?: string | null;
  sizeBytes?: number | null;
  episodesOwned?: number | null;
  episodesMonitored?: number | null;
  episodesTotal?: number | null;
  contentStatus?: string | null;
  posterUrl?: string | null;
  posterSourceUrl?: string | null;
  bannerUrl?: string | null;
  bannerSourceUrl?: string | null;
  backgroundUrl?: string | null;
  backgroundSourceUrl?: string | null;
  runtimeMinutes?: number | null;
  genres?: string[];
  language?: string | null;
  firstAired?: string | null;
  network?: string | null;
  studio?: string | null;
  country?: string | null;
  aliases?: string[];
  metadataLanguage?: string | null;
  metadataFetchedAt?: string | null;
  minAvailability?: string | null;
  qualityProfileId?: string | null;
  rootFolderPath?: string | null;
  monitorType?: string | null;
  useSeasonFolders?: boolean | null;
  monitorSpecials?: boolean | null;
  interSeasonMovies?: boolean | null;
  fillerPolicy?: string | null;
  recapPolicy?: string | null;
};

export type RootFolderOption = {
  path: string;
  isDefault: boolean;
};

export type LibraryScanSummary = {
  scanned: number;
  matched: number;
  imported: number;
  skipped: number;
  unmatched: number;
};
