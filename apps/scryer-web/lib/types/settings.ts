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
  pluginHttpCaBundlePem: string;
  pluginHttpTrustedCertificates: TrustedCertificateEntry[];
};

export type TrustedCertificateEntry = {
  fingerprintSha256: string;
  pem: string;
};

export type SecuritySettings = {
  formLoginEnabled: boolean;
  skipLoginForLocalIps: boolean;
  effectiveFormLoginEnabled: boolean;
  envOverrideActive: boolean;
  envOverrideDescription: string | null;
};

export type ExternalAccountProvider = "plex" | "jellyfin";
export type ExternalAccountStatus = "pending_claim" | "active" | "disabled";

export type AuthProviderConnection = {
  id: string;
  displayName: string;
  userVisibleUrl: string | null;
  baseUrl: string | null;
  machineId: string | null;
};

export type AuthProviderSettings = {
  allowedProviders: ExternalAccountProvider[];
  providerLoginEnabled: ExternalAccountProvider[];
  providerLinkingEnabled: ExternalAccountProvider[];
  allowedJellyfinConnectionIds: string[];
  allowedPlexConnectionIds: string[];
  allowedJellyfinConnections: AuthProviderConnection[];
  allowedPlexConnections: AuthProviderConnection[];
};

export type LinkedAccount = {
  id: string;
  userId: string;
  provider: ExternalAccountProvider;
  connectionId: string;
  externalUserId: string;
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
  status: ExternalAccountStatus;
  verifiedAt: string | null;
  createdAt: string;
  updatedAt: string;
};

export type AutoBackupSettings = {
  enabled: boolean;
  dailyTimeLocal: string;
  autoBackupKeyPresent: boolean;
  nextRunAt: string | null;
};

export type AuthRuntimeState = {
  effectiveFormLoginEnabled: boolean;
  skipLoginForLocalIps: boolean;
  passkeyEnabled: boolean;
};

export type PasskeySummary = {
  id: string;
  friendlyName: string | null;
  createdAt: string;
  lastUsedAt: string | null;
};

export type MediaSettings = {
  scope: "movie" | "series" | "anime";
  libraryPath: string;
  rootFolders: { path: string; isDefault: boolean }[];
  requiredAudioLanguages: string[];
  folderTemplate: string;
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
