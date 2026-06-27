import type { DiscoveryHomeInput, DiscoveryHomePayload } from "@/lib/types";

const DISCOVERY_HOME_CACHE_PREFIX = "scryer:discovery:dashboard-home:v1";

type NormalizedDiscoveryHomeInput = {
  includePublic: boolean | null;
  includePersonalized: boolean | null;
  includeUnresolved: boolean | null;
  limitPerSection: number | null;
};

type DiscoveryHomeCacheScope = {
  userId: string | null | undefined;
  uiLanguage: string;
  input: DiscoveryHomeInput;
};

type DiscoveryHomeCacheEntry = {
  userId: string;
  uiLanguage: string;
  input: NormalizedDiscoveryHomeInput;
  cachedAt: number;
  home: DiscoveryHomePayload;
};

function normalizeScopeValue(value: string | null | undefined) {
  const normalized = value?.trim() ?? "";
  return normalized || null;
}

function normalizeBoolean(value: boolean | null | undefined) {
  return typeof value === "boolean" ? value : null;
}

function normalizeNumber(value: number | null | undefined) {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function normalizeDiscoveryHomeInput(
  input: DiscoveryHomeInput,
): NormalizedDiscoveryHomeInput {
  return {
    includePublic: normalizeBoolean(input.includePublic),
    includePersonalized: normalizeBoolean(input.includePersonalized),
    includeUnresolved: normalizeBoolean(input.includeUnresolved),
    limitPerSection: normalizeNumber(input.limitPerSection),
  };
}

function discoveryHomeInputKey(input: DiscoveryHomeInput) {
  return JSON.stringify(normalizeDiscoveryHomeInput(input));
}

export function discoveryHomeCacheKey(scope: DiscoveryHomeCacheScope) {
  const userId = normalizeScopeValue(scope.userId);
  const uiLanguage = normalizeScopeValue(scope.uiLanguage);
  if (!userId || !uiLanguage) {
    return null;
  }

  return [
    DISCOVERY_HOME_CACHE_PREFIX,
    encodeURIComponent(userId),
    encodeURIComponent(uiLanguage),
    encodeURIComponent(discoveryHomeInputKey(scope.input)),
  ].join(":");
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isNormalizedDiscoveryHomeInput(
  value: unknown,
): value is NormalizedDiscoveryHomeInput {
  if (!isRecord(value)) {
    return false;
  }
  return (
    (typeof value.includePublic === "boolean" ||
      value.includePublic === null) &&
    (typeof value.includePersonalized === "boolean" ||
      value.includePersonalized === null) &&
    (typeof value.includeUnresolved === "boolean" ||
      value.includeUnresolved === null) &&
    (typeof value.limitPerSection === "number" ||
      value.limitPerSection === null)
  );
}

function normalizedInputsEqual(
  left: NormalizedDiscoveryHomeInput,
  right: NormalizedDiscoveryHomeInput,
) {
  return (
    left.includePublic === right.includePublic &&
    left.includePersonalized === right.includePersonalized &&
    left.includeUnresolved === right.includeUnresolved &&
    left.limitPerSection === right.limitPerSection
  );
}

function isDiscoverySyncStatus(value: unknown) {
  if (!isRecord(value) || !isRecord(value.state)) {
    return false;
  }
  return (
    typeof value.pendingContextChangeCount === "number" &&
    typeof value.state.updatedAt === "string"
  );
}

function isDiscoverySection(value: unknown) {
  return (
    isRecord(value) &&
    typeof value.sectionId === "string" &&
    typeof value.sectionType === "string" &&
    Array.isArray(value.items)
  );
}

function isDiscoveryHomePayload(value: unknown): value is DiscoveryHomePayload {
  return (
    isRecord(value) &&
    isDiscoverySyncStatus(value.status) &&
    Array.isArray(value.publicSections) &&
    Array.isArray(value.personalizedSections) &&
    (value.completeCollection === null ||
      isDiscoverySection(value.completeCollection)) &&
    Array.isArray(value.facets) &&
    typeof value.canViewPersonalized === "boolean"
  );
}

function isDiscoveryHomeCacheEntry(
  value: unknown,
  scope: DiscoveryHomeCacheScope,
): value is DiscoveryHomeCacheEntry {
  if (!isRecord(value)) {
    return false;
  }
  const userId = normalizeScopeValue(scope.userId);
  const uiLanguage = normalizeScopeValue(scope.uiLanguage);
  const expectedInput = normalizeDiscoveryHomeInput(scope.input);
  return (
    userId !== null &&
    uiLanguage !== null &&
    value.userId === userId &&
    value.uiLanguage === uiLanguage &&
    typeof value.cachedAt === "number" &&
    Number.isFinite(value.cachedAt) &&
    isNormalizedDiscoveryHomeInput(value.input) &&
    normalizedInputsEqual(value.input, expectedInput) &&
    isDiscoveryHomePayload(value.home)
  );
}

function removeStorageItem(key: string) {
  try {
    window.localStorage.removeItem(key);
  } catch {
    // Ignore persistence failures.
  }
}

export function readDiscoveryHomeCache(scope: DiscoveryHomeCacheScope) {
  const key = discoveryHomeCacheKey(scope);
  if (!key || typeof window === "undefined") {
    return null;
  }

  try {
    const raw = window.localStorage.getItem(key);
    if (!raw) {
      return null;
    }
    const parsed = JSON.parse(raw) as unknown;
    if (isDiscoveryHomeCacheEntry(parsed, scope)) {
      return parsed.home;
    }
    removeStorageItem(key);
  } catch {
    removeStorageItem(key);
  }
  return null;
}

export function writeDiscoveryHomeCache(
  scope: DiscoveryHomeCacheScope,
  home: DiscoveryHomePayload,
) {
  const key = discoveryHomeCacheKey(scope);
  const userId = normalizeScopeValue(scope.userId);
  const uiLanguage = normalizeScopeValue(scope.uiLanguage);
  if (!key || !userId || !uiLanguage || typeof window === "undefined") {
    return;
  }

  try {
    window.localStorage.setItem(
      key,
      JSON.stringify({
        userId,
        uiLanguage,
        input: normalizeDiscoveryHomeInput(scope.input),
        cachedAt: Date.now(),
        home,
      } satisfies DiscoveryHomeCacheEntry),
    );
  } catch {
    // Ignore persistence failures.
  }
}

export function clearDiscoveryHomeCache(scope: DiscoveryHomeCacheScope) {
  const key = discoveryHomeCacheKey(scope);
  if (!key || typeof window === "undefined") {
    return;
  }
  removeStorageItem(key);
}
