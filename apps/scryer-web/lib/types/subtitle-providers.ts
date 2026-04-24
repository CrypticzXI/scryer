import type { ProviderTypeInfo } from "./indexers";

export type SubtitleProviderConfigRecord = {
  id: string;
  name: string;
  providerType: string;
  hasConfig: boolean;
  storedSecretKeys: string[];
  enabledFacets: Array<"movie" | "series" | "anime">;
  isEnabled: boolean;
  lastHealthStatus: string | null;
  lastError: string | null;
  lastErrorAt: string | null;
  disabledUntil: string | null;
  createdAt: string;
  updatedAt: string;
};

export type SubtitleProviderDraft = {
  name: string;
  providerType: string;
  isEnabled: boolean;
  enabledFacets: Array<"movie" | "series" | "anime">;
  configValues: Record<string, string>;
  persistedConfigValues: Record<string, string>;
  storedSecretKeys: string[];
  configDirty: boolean;
};

export type SubtitleProviderValidationResult = {
  status: string;
  message: string | null;
  retryAfterSeconds: number | null;
};

export type SubtitleProviderTypeInfo = ProviderTypeInfo;
