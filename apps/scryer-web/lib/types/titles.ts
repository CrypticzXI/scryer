export type Facet = "movie" | "series" | "anime";

export type ExternalId = {
  source: string;
  value: string;
};

export type TitleRecord = {
  id: string;
  name: string;
  facet: Facet;
  libraryId: string;
  libraryName?: string | null;
  librarySlug?: string | null;
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

export type LibraryRootRecord = {
  id: string;
  path: string;
  isDefault: boolean;
};

export type LibraryRecord = {
  id: string;
  facet: Facet;
  name: string;
  slug: string;
  isDefault: boolean;
  roots: LibraryRootRecord[];
};

export type LibrarySettingsRecord = {
  requiredAudioLanguagesOverride: string[] | null;
  requiredAudioLanguages: string[];
  qualityProfileIdOverride: string | null;
  qualityProfileId: string;
  scoringPersonaOverride: string | null;
  scoringPersona: string;
  indexerRoutingOverride: unknown[] | null;
  downloadClientRoutingOverride: unknown[] | null;
};

export type LibrarySettingsDraft = {
  requiredAudioLanguages: string[] | null;
  qualityProfileId: string | null;
  scoringPersona: string | null;
  indexerRouting?: unknown[] | null;
  downloadClientRouting?: unknown[] | null;
};

export type LibraryScanSummary = {
  scanned: number;
  matched: number;
  imported: number;
  skipped: number;
  unmatched: number;
};
