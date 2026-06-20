import type { DownloadClientRoutingEntry } from "./download-clients";
import type { ImportMode } from "./settings";

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
  externalIds?: ExternalId[] | null;
  qualityTier?: string | null;
  currentQualityTier?: string | null;
  sizeBytes?: number | null;
  episodesOwned?: number | null;
  episodesMonitored?: number | null;
  episodesTotal?: number | null;
  contentStatus?: string | null;
  posterUrl?: string | null;
  posterSourceUrl?: string | null;
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
  rootFolderId?: string | null;
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
  qualityProfileId?: string | null;
  requestQualityProfileIds?: string[];
  requestQualityProfileDefaultId?: string | null;
};

export type MediaRequestRequesterRecord = {
  userId: string;
  username: string;
  avatarUrl?: string | null;
  requestedAt: string;
};

export type MediaRequestRecord = {
  id: string;
  libraryId: string;
  facet: Facet;
  status: "pending" | "approved" | "rejected" | "canceled";
  identityFingerprint: string;
  title: string;
  sortTitle?: string | null;
  slug?: string | null;
  posterUrl?: string | null;
  year?: number | null;
  overview?: string | null;
  runtimeMinutes?: number | null;
  language?: string | null;
  contentStatus?: string | null;
  requestedQualityProfileId?: string | null;
  requestedQualityProfileName?: string | null;
  requestedMonitorType?: string | null;
  resolvedByUserId?: string | null;
  resolvedAt?: string | null;
  createdTitleId?: string | null;
  approvedQualityProfileId?: string | null;
  approvedQualityProfileName?: string | null;
  externalIds: ExternalId[];
  requesters: MediaRequestRequesterRecord[];
  createdByUserId: string;
  createdAt: string;
  updatedAt: string;
};

export type LibrarySettingsRecord = {
  requiredAudioLanguagesOverride: string[] | null;
  requiredAudioLanguages: string[];
  qualityProfileIdOverride: string | null;
  qualityProfileId: string;
  requestQualityProfileIdsOverride: string[] | null;
  requestQualityProfileIds: string[];
  requestQualityProfileDefaultId: string;
  scoringPersonaOverride: string | null;
  scoringPersona: string;
  fillerPolicyOverride: string | null;
  fillerPolicy: string | null;
  recapPolicyOverride: string | null;
  recapPolicy: string | null;
  monitorSpecialsOverride: boolean | null;
  monitorSpecials: boolean | null;
  interSeasonMoviesOverride: boolean | null;
  interSeasonMovies: boolean | null;
  monitorFillerMoviesOverride: boolean | null;
  monitorFillerMovies: boolean | null;
  nfoWriteOnImportOverride: boolean | null;
  nfoWriteOnImport: boolean;
  plexmatchWriteOnImportOverride: boolean | null;
  plexmatchWriteOnImport: boolean | null;
  importModeOverride: ImportMode | null;
  importMode: ImportMode;
  indexerRoutingOverride: unknown[] | null;
  downloadClientRoutingOverride: DownloadClientRoutingEntry[] | null;
};

export type LibrarySettingsDraft = {
  requiredAudioLanguages: string[] | null;
  qualityProfileId: string | null;
  requestQualityProfileIds: string[] | null;
  scoringPersona: string | null;
  fillerPolicy: string | null;
  recapPolicy: string | null;
  monitorSpecials: boolean | null;
  interSeasonMovies: boolean | null;
  monitorFillerMovies: boolean | null;
  nfoWriteOnImport: boolean | null;
  plexmatchWriteOnImport: boolean | null;
  importMode: ImportMode | null;
  indexerRouting?: unknown[] | null;
  downloadClientRouting?: DownloadClientRoutingEntry[] | null;
};

export type LibraryScanSummary = {
  scanned: number;
  matched: number;
  imported: number;
  skipped: number;
  unmatched: number;
};
