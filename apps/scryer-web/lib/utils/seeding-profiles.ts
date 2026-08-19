import type {
  SeasonPackSeedMode,
  SeedGoalMetAction,
  SeedingProfileDraft,
  SeedingProfileRecord,
} from "@/lib/types/seeding-profiles";

/** Settings key backing the global default seeding profile. */
export const DEFAULT_SEEDING_PROFILE_SETTING_KEY =
  "download_client.default_seeding_profile";

/**
 * Sentinel select value for "no profile". Radix selects cannot carry an empty
 * string value, so assignment dropdowns use this the way the download-client
 * mapping dropdown uses `AUTOMATIC_DOWNLOAD_CLIENT_ID`.
 */
export const SEEDING_PROFILE_INHERIT_VALUE = "__inherit__";

export const SEASON_PACK_SEED_MODES: readonly SeasonPackSeedMode[] = [
  "INHERIT",
  "OVERRIDE",
];

export const SEED_GOAL_MET_ACTIONS: readonly SeedGoalMetAction[] = [
  "REMOVE_ENTRY",
  "STOP_SEEDING",
  "KEEP",
];

export function buildSeedingProfileTemplate(): SeedingProfileDraft {
  return {
    id: "",
    name: "",
    ratio: "",
    seedTimeMinutes: "",
    seasonPackMode: "INHERIT",
    seasonPackRatio: "",
    seasonPackSeedTimeMinutes: "",
    honorTrackerMinimums: true,
    goalMetAction: "REMOVE_ENTRY",
    neverRemove: false,
  };
}

function numberToDraftValue(value: number | null | undefined): string {
  return value === null || value === undefined ? "" : String(value);
}

export function seedingProfileToDraft(
  profile: SeedingProfileRecord,
): SeedingProfileDraft {
  return {
    id: profile.id,
    name: profile.name,
    ratio: numberToDraftValue(profile.ratio),
    seedTimeMinutes: numberToDraftValue(profile.seedTimeMinutes),
    seasonPackMode: profile.seasonPackMode,
    seasonPackRatio: numberToDraftValue(profile.seasonPackRatio),
    seasonPackSeedTimeMinutes: numberToDraftValue(
      profile.seasonPackSeedTimeMinutes,
    ),
    honorTrackerMinimums: profile.honorTrackerMinimums,
    goalMetAction: profile.goalMetAction,
    neverRemove: profile.neverRemove,
  };
}

type ParsedGoal =
  | { ok: true; value: number | null }
  | { ok: false };

function parseOptionalRatio(raw: string): ParsedGoal {
  const trimmed = raw.trim();
  if (!trimmed) {
    return { ok: true, value: null };
  }
  const parsed = Number(trimmed);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    return { ok: false };
  }
  return { ok: true, value: parsed };
}

function parseOptionalMinutes(raw: string): ParsedGoal {
  const trimmed = raw.trim();
  if (!trimmed) {
    return { ok: true, value: null };
  }
  if (!/^\d+$/.test(trimmed)) {
    return { ok: false };
  }
  const parsed = Number(trimmed);
  if (!Number.isSafeInteger(parsed) || parsed <= 0) {
    return { ok: false };
  }
  return { ok: true, value: parsed };
}

/**
 * Mirrors the backend's `SeedingProfile::validate` so bad input is rejected
 * before the mutation round-trip. Returns a message key-free English string,
 * matching `validateDelayProfileDraft`.
 */
export function validateSeedingProfileDraft(
  draft: SeedingProfileDraft,
): string | null {
  if (!draft.name.trim()) {
    return "Seeding profile name is required.";
  }
  if (!parseOptionalRatio(draft.ratio).ok) {
    return "Ratio must be a number greater than zero, or empty.";
  }
  if (!parseOptionalMinutes(draft.seedTimeMinutes).ok) {
    return "Seed time must be a whole number of minutes greater than zero, or empty.";
  }
  if (draft.seasonPackMode === "OVERRIDE") {
    if (!parseOptionalRatio(draft.seasonPackRatio).ok) {
      return "Season-pack ratio must be a number greater than zero, or empty.";
    }
    if (!parseOptionalMinutes(draft.seasonPackSeedTimeMinutes).ok) {
      return "Season-pack seed time must be a whole number of minutes greater than zero, or empty.";
    }
  }
  return null;
}

type SeedingProfileGoalInput = {
  name: string;
  ratio: number | null;
  seedTimeMinutes: number | null;
  seasonPackMode: SeasonPackSeedMode;
  seasonPackRatio: number | null;
  seasonPackSeedTimeMinutes: number | null;
  honorTrackerMinimums: boolean;
  goalMetAction: SeedGoalMetAction;
  neverRemove: boolean;
};

/**
 * Normalizes a validated draft into mutation-input shape. Season-pack goals are
 * dropped in inherit mode, mirroring `SeedingProfile::normalized()` server-side
 * so the optimistic row matches what comes back.
 */
export function seedingProfileDraftToInput(
  draft: SeedingProfileDraft,
): SeedingProfileGoalInput {
  const ratio = parseOptionalRatio(draft.ratio);
  const seedTime = parseOptionalMinutes(draft.seedTimeMinutes);
  const seasonPackRatio = parseOptionalRatio(draft.seasonPackRatio);
  const seasonPackSeedTime = parseOptionalMinutes(
    draft.seasonPackSeedTimeMinutes,
  );
  const isOverride = draft.seasonPackMode === "OVERRIDE";

  return {
    name: draft.name.trim(),
    ratio: ratio.ok ? ratio.value : null,
    seedTimeMinutes: seedTime.ok ? seedTime.value : null,
    seasonPackMode: draft.seasonPackMode,
    seasonPackRatio: isOverride && seasonPackRatio.ok ? seasonPackRatio.value : null,
    seasonPackSeedTimeMinutes:
      isOverride && seasonPackSeedTime.ok ? seasonPackSeedTime.value : null,
    honorTrackerMinimums: draft.honorTrackerMinimums,
    goalMetAction: draft.goalMetAction,
    neverRemove: draft.neverRemove,
  };
}

export function toCreateSeedingProfileInput(draft: SeedingProfileDraft) {
  return seedingProfileDraftToInput(draft);
}

/**
 * Every optional goal is sent explicitly so clearing a field in the editor
 * clears it server-side — `UpdateSeedingProfileInput` treats an explicit null
 * as "clear" and an omitted key as "preserve".
 */
export function toUpdateSeedingProfileInput(draft: SeedingProfileDraft) {
  return { id: draft.id, ...seedingProfileDraftToInput(draft) };
}

/** Reads the raw server message off a urql error without rewording it. */
export function extractSeedingProfileErrorMessage(error: unknown): string | null {
  if (error && typeof error === "object" && "graphQLErrors" in error) {
    const graphQLErrors = (
      error as { graphQLErrors?: Array<{ message?: string }> }
    ).graphQLErrors;
    const message = graphQLErrors?.find(
      (entry) => typeof entry.message === "string" && entry.message.trim(),
    )?.message;
    if (message?.trim()) {
      return message.trim();
    }
  }

  if (error instanceof Error && error.message.trim()) {
    return error.message.trim();
  }

  return null;
}

const EM_DASH = "—";

export function formatSeedingProfileRatio(ratio: number | null): string {
  return ratio === null ? EM_DASH : String(ratio);
}

export function formatSeedingProfileSeedTime(minutes: number | null): string {
  return minutes === null ? EM_DASH : `${minutes}m`;
}

/**
 * One-cell summary of the season-pack column: the override goals when the
 * profile overrides, otherwise the inherit marker.
 */
export function formatSeasonPackSummary(
  profile: Pick<
    SeedingProfileRecord,
    "seasonPackMode" | "seasonPackRatio" | "seasonPackSeedTimeMinutes"
  >,
  inheritLabel: string,
): string {
  if (profile.seasonPackMode !== "OVERRIDE") {
    return inheritLabel;
  }
  const parts = [
    profile.seasonPackRatio === null ? null : String(profile.seasonPackRatio),
    profile.seasonPackSeedTimeMinutes === null
      ? null
      : `${profile.seasonPackSeedTimeMinutes}m`,
  ].filter((part): part is string => part !== null);
  return parts.length === 0 ? EM_DASH : parts.join(" / ");
}

/** Resolves a stored assignment id to a select value, tolerating stale ids. */
export function seedingProfileSelectValue(
  seedingProfileId: string | null | undefined,
): string {
  return seedingProfileId ?? SEEDING_PROFILE_INHERIT_VALUE;
}

export function seedingProfileSelectValueToId(value: string): string | null {
  return value === SEEDING_PROFILE_INHERIT_VALUE ? null : value;
}

/**
 * True when an indexer can carry a seeding profile at all. The backend rejects
 * assignment on anything that is not torrent-capable, so the control mirrors
 * the download-client mapping's not-applicable handling instead of failing the
 * mutation.
 */
export function supportsSeedingProfileAssignment(
  protocolFamilies: readonly string[] | undefined,
): boolean {
  return (protocolFamilies ?? []).some(
    (family) => family.trim().toLowerCase() === "torrent",
  );
}
