export type SubtitleDownloadRecord = {
  id: string;
  mediaFileId: string;
  language: string;
  provider: string;
  filePath: string;
  score: number | null;
  hearingImpaired: boolean;
  forced: boolean;
  aiTranslated: boolean;
  machineTranslated: boolean;
  uploader: string | null;
  releaseInfo: string | null;
  synced: boolean;
  downloadedAt: string;
};

export type SubtitleBlacklistEntryRecord = {
  id: string;
  mediaFileId: string;
  provider: string;
  providerFileId: string;
  language: string;
  reason: string | null;
  createdAt: string;
};
