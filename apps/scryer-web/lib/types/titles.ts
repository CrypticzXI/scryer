import type { DownloadClientRoutingEntry } from "./download-clients";
import type { CatalogDiscoveryItem } from "./discovery";
import type { ImportMode } from "./settings";

export type Facet = "movie" | "series" | "anime";

export type ExternalId = {
  source: string;
  value: string;
};

export type TitleCollectionEpisodeRecord = {
  id: string;
  titleId: string;
  collectionId?: string | null;
  episodeType?: string | null;
  episodeNumber?: string | number | null;
  seasonNumber?: string | number | null;
  episodeLabel?: string | null;
  title?: string | null;
  overview?: string | null;
  airDate?: string | null;
  durationSeconds?: number | null;
  hasMultiAudio?: boolean | null;
  hasSubtitle?: boolean | null;
  isFiller?: boolean | null;
  isRecap?: boolean | null;
  absoluteNumber?: string | number | null;
  imageUrl?: string | null;
  monitored?: boolean | null;
  createdAt?: string | null;
};

export type TitleCollectionRecord = {
  id: string;
  titleId: string;
  collectionType?: string | null;
  collectionIndex?: string | number | null;
  label?: string | null;
  orderedPath?: string | null;
  narrativeOrder?: string | number | null;
  fileSizeBytes?: number | null;
  firstEpisodeNumber?: string | number | null;
  lastEpisodeNumber?: string | number | null;
  monitored?: boolean | null;
  episodes?: TitleCollectionEpisodeRecord[] | null;
  createdAt?: string | null;
};

export type TitleMediaFileRecord = {
  id: string;
  titleId: string;
  episodeId?: string | null;
  seriesMovieLinkIds?: string[] | null;
  role?: string | null;
  filePath?: string | null;
  sizeBytes?: number | null;
  qualityLabel?: string | null;
  scanStatus?: string | null;
  createdAt?: string | null;
  videoCodec?: string | null;
  videoWidth?: number | null;
  videoHeight?: number | null;
  videoBitrateKbps?: number | null;
  videoBitDepth?: number | null;
  videoHdrFormat?: string | null;
  videoFrameRate?: string | null;
  videoProfile?: string | null;
  audioCodec?: string | null;
  audioChannels?: number | null;
  audioBitrateKbps?: number | null;
  audioLanguages?: string[] | null;
  audioStreams?:
    | {
        codec: string | null;
        channels: number | null;
        language: string | null;
        bitrateKbps: number | null;
      }[]
    | null;
  subtitleLanguages?: string[] | null;
  subtitleCodecs?: string[] | null;
  subtitleStreams?:
    | {
        codec: string | null;
        language: string | null;
        name: string | null;
        forced: boolean | null;
        default: boolean | null;
      }[]
    | null;
  hasMultiaudio?: boolean | null;
  durationSeconds?: number | null;
  numChapters?: number | null;
  containerFormat?: string | null;
  sceneName?: string | null;
  releaseGroup?: string | null;
  sourceType?: string | null;
  resolution?: string | null;
  videoCodecParsed?: string | null;
  audioCodecParsed?: string | null;
  acquisitionScore?: number | null;
  scoringLog?: string | null;
  indexerSource?: string | null;
  grabbedReleaseTitle?: string | null;
  grabbedAt?: string | null;
  edition?: string | null;
  originalFilePath?: string | null;
  releaseHash?: string | null;
};

export type TitleReleaseBlocklistEntry = {
  id: string;
  sourceHint: string | null;
  sourceTitle: string | null;
  errorMessage: string | null;
  attemptedAt: string;
  episodeIds: string[];
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
  rootFolderId?: string;
  rootFolderPath?: string;
  monitorType?: string | null;
  useSeasonFolders?: boolean | null;
  monitorSpecials?: boolean | null;
  interSeasonMovies?: boolean | null;
  fillerPolicy?: string | null;
  recapPolicy?: string | null;
  collections?: TitleCollectionRecord[] | null;
  mediaFiles?: TitleMediaFileRecord[] | null;
  moreLikeThis?: CatalogDiscoveryItem[] | null;
};

export type RootFolderOption = {
  id?: string;
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
  setPermissionsLinuxOverride: boolean | null;
  setPermissionsLinux: boolean;
  fileChmodOverride: string | null;
  fileChmod: string | null;
  folderChmodOverride: string | null;
  folderChmod: string | null;
  chownGroupOverride: string | null;
  chownGroup: string | null;
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
  setPermissionsLinux: boolean | null;
  fileChmod: string | null;
  folderChmod: string | null;
  chownGroup: string | null;
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
