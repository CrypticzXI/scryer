export type DownloadSourceKind = "NZB_FILE" | "NZB_URL" | "TORRENT_FILE" | "MAGNET_URI";

export type ReleaseQueueScope = {
  kind: "EPISODE" | "episode_set" | "collection" | "SERIES_MOVIE" | "TITLE" | "orphan";
  episodeId?: string | null;
  episodeIds: string[];
  collectionId?: string | null;
  seriesMovieLinkId?: string | null;
};

export type Release = {
  source: string | null;
  title: string;
  link: string | null;
  downloadUrl: string | null;
  candidateToken?: string | null;
  queueScope?: ReleaseQueueScope | null;
  sourceKind?: DownloadSourceKind | null;
  sizeBytes: number | null;
  publishedAt: string | null;
  thumbsUp?: number | null;
  thumbsDown?: number | null;
  parsedRelease?: {
    rawTitle: string;
    normalizedTitle: string;
    releaseGroup?: string | null;
    quality?: string | null;
    source?: string | null;
    videoCodec?: string | null;
    videoEncoding?: string | null;
    audio?: string | null;
    isDualAudio: boolean;
    isAtmos: boolean;
    isDolbyVision: boolean;
    detectedHdr: boolean;
    parseConfidence: number;
    isProperUpload: boolean;
    isRemux: boolean;
    isBdDisk: boolean;
    isAiEnhanced: boolean;
  } | null;
  qualityProfileDecision?: {
    allowed: boolean;
    blockCodes: string[];
    releaseScore: number;
    preferenceScore: number;
    scoringLog: { code: string; delta: number; source: string; ruleSetName?: string | null }[];
  } | null;
  autoEligible?: boolean | null;
  autoDecisionCode?: string | null;
  autoDecisionSummary?: string | null;
};
