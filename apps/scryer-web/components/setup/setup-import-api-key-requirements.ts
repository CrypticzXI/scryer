import type { ConfigFieldDef } from "@/lib/types";
import type {
  ExternalImportDownloadClient,
  ExternalImportIndexer,
  ExternalImportPreview,
} from "@/lib/types/external-import";

const IMPORT_DOWNLOAD_CLIENT_TYPES_REQUIRING_API_KEY = new Set([
  "sabnzbd",
  "weaver",
]);

export function providerRequiresApiKey(fields: ConfigFieldDef[]): boolean {
  return fields.some((field) => {
    const normalizedKey = field.key.trim().toLowerCase();
    const normalizedFieldType = field.fieldType.trim().toLowerCase();
    return field.required && (
      normalizedKey === "api_key" ||
      normalizedKey === "apikey" ||
      (
        (normalizedFieldType === "password" || normalizedFieldType === "secret")
        && normalizedKey.includes("api")
      )
    );
  });
}

export function externalImportDownloadClientNeedsUserSuppliedApiKey(
  downloadClient: ExternalImportDownloadClient,
): boolean {
  const normalizedClientType = downloadClient.scryerClientType?.trim().toLowerCase() ?? null;
  return (
    downloadClient.supported &&
    downloadClient.apiKey === null &&
    normalizedClientType !== null &&
    IMPORT_DOWNLOAD_CLIENT_TYPES_REQUIRING_API_KEY.has(normalizedClientType)
  );
}

export function externalImportIndexerNeedsUserSuppliedApiKey(
  indexer: ExternalImportIndexer,
  providerConfigFields: ConfigFieldDef[],
): boolean {
  return indexer.supported && (
    indexer.requiresApiKeyOverride ||
    (indexer.apiKey === null && providerRequiresApiKey(providerConfigFields))
  );
}

export type MissingExternalImportApiKeyRequirement = {
  isProwlarr: boolean;
  name: string;
};

export function findMissingExternalImportApiKeyRequirement({
  preview,
  selectedDcKeys,
  selectedIdxKeys,
  dcApiKeyOverrides,
  idxApiKeyOverrides,
  indexerProviderConfigFieldsByType,
}: {
  preview: ExternalImportPreview;
  selectedDcKeys: Set<string>;
  selectedIdxKeys: Set<string>;
  dcApiKeyOverrides: Map<string, string>;
  idxApiKeyOverrides: Map<string, string>;
  indexerProviderConfigFieldsByType: Map<string, ConfigFieldDef[]>;
}): MissingExternalImportApiKeyRequirement | null {
  const missingDownloadClient = preview.downloadClients.find((downloadClient) => (
    selectedDcKeys.has(downloadClient.dedupKey) &&
    externalImportDownloadClientNeedsUserSuppliedApiKey(downloadClient) &&
    !(dcApiKeyOverrides.get(downloadClient.dedupKey)?.trim())
  ));
  if (missingDownloadClient) {
    return {
      isProwlarr: false,
      name: missingDownloadClient.name,
    };
  }

  const missingIndexer = preview.indexers.find((indexer) => {
    const providerConfigFields =
      indexer.scryerProviderType === null
        ? []
        : (indexerProviderConfigFieldsByType.get(indexer.scryerProviderType) ?? []);
    return (
      selectedIdxKeys.has(indexer.dedupKey) &&
      externalImportIndexerNeedsUserSuppliedApiKey(indexer, providerConfigFields) &&
      !(idxApiKeyOverrides.get(indexer.dedupKey)?.trim())
    );
  });
  if (missingIndexer) {
    return {
      isProwlarr: missingIndexer.scryerProviderType === "prowlarr",
      name: missingIndexer.name,
    };
  }

  return null;
}
