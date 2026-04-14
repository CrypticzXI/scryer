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
  imdbId?: string | null;
  externalIds: ExternalId[];
  qualityTier?: string | null;
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
