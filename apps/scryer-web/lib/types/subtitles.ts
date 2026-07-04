export type ExternalSubtitleSourceKind = "downloaded" | "discovered";

export type ExternalSubtitleRecord = {
  id: string;
  mediaFileId: string;
  titleId: string;
  episodeId: string | null;
  sourceKind: ExternalSubtitleSourceKind;
  language: string;
  provider: string | null;
  providerFileId: string | null;
  filePath: string;
  score: number | null;
  scorePercent: number | null;
  hearingImpaired: boolean;
  forced: boolean;
  aiTranslated: boolean;
  machineTranslated: boolean;
  uploader: string | null;
  releaseInfo: string | null;
  synced: boolean;
  downloadedAt: string;
};

export type ExternalSubtitleBlocklistEntryRecord = {
  id: string;
  mediaFileId: string;
  provider: string;
  providerFileId: string;
  language: string;
  reason: string | null;
  createdAt: string;
};
