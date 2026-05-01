export type SubtitleLanguagePreference = {
  code: string;
  hearingImpaired: boolean;
  forced: boolean;
};

export type SubtitleSettings = {
  enabled: boolean;
  languages: SubtitleLanguagePreference[];
  autoDownloadOnImport: boolean;
  minimumScoreSeries: number;
  minimumScoreMovie: number;
  searchIntervalHours: number;
  includeAiTranslated: boolean;
  includeMachineTranslated: boolean;
  syncEnabled: boolean;
  syncThresholdSeries: number;
  syncThresholdMovie: number;
  syncMaxOffsetSeconds: number;
};

export type AcquisitionSettings = {
  enabled: boolean;
  upgradeCooldownHours: number;
  sameTierMinDelta: number;
  crossTierMinDelta: number;
  forcedUpgradeDeltaBypass: number;
  pollIntervalSeconds: number;
  syncIntervalSeconds: number;
  batchSize: number;
};

export type GeneralSettings = {
  keepHistoryForever: boolean;
  historyRetentionDays: number;
};

export type SecuritySettings = {
  formLoginEnabled: boolean;
  skipLoginForLocalIps: boolean;
  effectiveFormLoginEnabled: boolean;
  envOverrideActive: boolean;
  envOverrideDescription: string | null;
};

export type AuthRuntimeState = {
  effectiveFormLoginEnabled: boolean;
  skipLoginForLocalIps: boolean;
};

export type MediaSettings = {
  scope: "movie" | "series" | "anime";
  libraryPath: string;
  rootFolders: { path: string; isDefault: boolean }[];
  requiredAudioLanguages: string[];
  renameTemplate: string;
  renameCollisionPolicy: string;
  renameMissingMetadataPolicy: string;
  fillerPolicy: string | null;
  recapPolicy: string | null;
  monitorSpecials: boolean | null;
  interSeasonMovies: boolean | null;
  monitorFillerMovies: boolean | null;
  nfoWriteOnImport: boolean;
  plexmatchWriteOnImport: boolean | null;
};

export type LibraryPaths = {
  moviePath: string;
  seriesPath: string;
  animePath: string;
};

export type ServiceSettings = {
  tlsCertPath: string;
  tlsKeyPath: string;
};
