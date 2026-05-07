import type { ViewCategoryId } from "./quality-profiles";

export type IndexerRecord = {
  id: string;
  name: string;
  providerType: string;
  baseUrl: string;
  hasApiKey: boolean;
  storedSecretKeys: string[];
  rateLimitSeconds: number | null;
  rateLimitBurst: number | null;
  disabledUntil: string | null;
  isEnabled: boolean;
  enableInteractiveSearch: boolean;
  enableAutoSearch: boolean;
  lastHealthStatus: string | null;
  lastErrorAt: string | null;
  lastQueryAt: string | null;
  configJson: string | null;
  createdAt: string;
  updatedAt: string;
};

export type IndexerDraft = {
  name: string;
  providerType: string;
  storedSecretKeys: string[];
  isEnabled: boolean;
  enableInteractiveSearch: boolean;
  enableAutoSearch: boolean;
  configValues: Record<string, string>;
};

export type ConfigFieldOption = {
  value: string;
  label: string;
};

export type ConfigFieldDef = {
  key: string;
  label: string;
  fieldType: string;
  required: boolean;
  defaultValue: string | null;
  valueSource: "user" | "host_binding";
  role: "connection_url" | null;
  hostBinding: string | null;
  options: ConfigFieldOption[];
  helpText: string | null;
};

export type ProviderTypeInfo = {
  providerType: string;
  name: string;
  defaultBaseUrl: string | null;
  configFields: ConfigFieldDef[];
  availableHostBindings: string[];
  recommendedFacets: Array<"movie" | "series" | "anime">;
};

export function visibleIndexerConfigFields(
  providerType: string,
  configFields: ConfigFieldDef[],
): ConfigFieldDef[] {
  const normalizedProviderType = providerType.trim().toLowerCase();
  if (normalizedProviderType === "nzbgeek") {
    return configFields.filter(
      (field) => field.key !== "base_url" && field.key !== "api_path",
    );
  }
  return configFields;
}

export type IndexerCategoryRoutingSettings = {
  categories: string[];
  enabled: boolean;
  priority: number;
};

export type IndexerRoutingEntry = {
  indexerId: string;
  enabled: boolean;
  categories: string[];
  priority: number;
};

export type IndexerRoutingSettingsByIndexer = Record<string, IndexerCategoryRoutingSettings>;

export type IndexerRoutingSettingsByScope = Record<ViewCategoryId, IndexerRoutingSettingsByIndexer>;
