import assert from "node:assert/strict";
import test from "node:test";

import en from "../i18n/locales/en.ts";
import type { SeedingProfileDraft } from "../types/seeding-profiles.ts";
import {
  buildSeedingProfileTemplate,
  extractSeedingProfileErrorMessage,
  formatSeasonPackSummary,
  formatSeedingProfileRatio,
  formatSeedingProfileSeedTime,
  SEEDING_PROFILE_INHERIT_VALUE,
  seedingProfileSelectValue,
  seedingProfileSelectValueToId,
  seedingProfileToDraft,
  supportsSeedingProfileAssignment,
  toCreateSeedingProfileInput,
  toUpdateSeedingProfileInput,
  validateSeedingProfileDraft,
} from "./seeding-profiles.ts";

function draft(overrides: Partial<SeedingProfileDraft> = {}): SeedingProfileDraft {
  return { ...buildSeedingProfileTemplate(), name: "Private", ...overrides };
}

test("template opts in to nothing but the safe defaults", () => {
  const template = buildSeedingProfileTemplate();
  assert.equal(template.ratio, "");
  assert.equal(template.seedTimeMinutes, "");
  assert.equal(template.seasonPackMode, "INHERIT");
  assert.equal(template.honorTrackerMinimums, true);
  assert.equal(template.goalMetAction, "REMOVE_ENTRY");
  assert.equal(template.neverRemove, false);
});

test("a stored profile round-trips through the draft", () => {
  const record = {
    id: "profile-1",
    name: "Private",
    ratio: 1.5,
    seedTimeMinutes: 4320,
    seasonPackMode: "OVERRIDE" as const,
    seasonPackRatio: 2,
    seasonPackSeedTimeMinutes: 10080,
    honorTrackerMinimums: false,
    goalMetAction: "STOP_SEEDING" as const,
    neverRemove: true,
  };

  const asDraft = seedingProfileToDraft(record);
  assert.equal(asDraft.ratio, "1.5");
  assert.equal(asDraft.seasonPackSeedTimeMinutes, "10080");

  assert.deepEqual(toUpdateSeedingProfileInput(asDraft), {
    id: "profile-1",
    name: "Private",
    ratio: 1.5,
    seedTimeMinutes: 4320,
    seasonPackMode: "OVERRIDE",
    seasonPackRatio: 2,
    seasonPackSeedTimeMinutes: 10080,
    honorTrackerMinimums: false,
    goalMetAction: "STOP_SEEDING",
    neverRemove: true,
  });
});

test("empty goal fields become nulls so the client's own limits apply", () => {
  const input = toCreateSeedingProfileInput(draft());
  assert.equal(input.ratio, null);
  assert.equal(input.seedTimeMinutes, null);
  assert.equal(input.seasonPackRatio, null);
  assert.equal(input.seasonPackSeedTimeMinutes, null);
});

test("inherit mode drops season-pack goals, matching server normalization", () => {
  const input = toCreateSeedingProfileInput(
    draft({
      seasonPackMode: "INHERIT",
      seasonPackRatio: "3",
      seasonPackSeedTimeMinutes: "60",
    }),
  );
  assert.equal(input.seasonPackRatio, null);
  assert.equal(input.seasonPackSeedTimeMinutes, null);
});

test("override mode keeps season-pack goals", () => {
  const input = toCreateSeedingProfileInput(
    draft({
      seasonPackMode: "OVERRIDE",
      seasonPackRatio: "3",
      seasonPackSeedTimeMinutes: "60",
    }),
  );
  assert.equal(input.seasonPackRatio, 3);
  assert.equal(input.seasonPackSeedTimeMinutes, 60);
});

test("validation rejects what the backend rejects", () => {
  assert.equal(validateSeedingProfileDraft(draft()), null);
  assert.match(
    validateSeedingProfileDraft(draft({ name: "  " })) ?? "",
    /name is required/,
  );
  assert.match(
    validateSeedingProfileDraft(draft({ ratio: "0" })) ?? "",
    /Ratio must be/,
  );
  assert.match(
    validateSeedingProfileDraft(draft({ ratio: "-1" })) ?? "",
    /Ratio must be/,
  );
  assert.match(
    validateSeedingProfileDraft(draft({ ratio: "abc" })) ?? "",
    /Ratio must be/,
  );
  assert.match(
    validateSeedingProfileDraft(draft({ seedTimeMinutes: "1.5" })) ?? "",
    /Seed time must be/,
  );
  assert.equal(validateSeedingProfileDraft(draft({ ratio: "1.25" })), null);
});

test("season-pack goals are only validated in override mode", () => {
  assert.equal(
    validateSeedingProfileDraft(
      draft({ seasonPackMode: "INHERIT", seasonPackRatio: "nonsense" }),
    ),
    null,
  );
  assert.match(
    validateSeedingProfileDraft(
      draft({ seasonPackMode: "OVERRIDE", seasonPackRatio: "nonsense" }),
    ) ?? "",
    /Season-pack ratio must be/,
  );
});

test("blocked-delete messages are surfaced verbatim", () => {
  const blocked =
    "seeding profile is still assigned to indexer 'Private', the global default seeding profile";
  assert.equal(
    extractSeedingProfileErrorMessage({
      graphQLErrors: [{ message: blocked }],
    }),
    blocked,
  );
  assert.equal(
    extractSeedingProfileErrorMessage(new Error("network down")),
    "network down",
  );
  assert.equal(extractSeedingProfileErrorMessage({}), null);
});

test("null assignments map to the inherit sentinel and back", () => {
  assert.equal(seedingProfileSelectValue(null), SEEDING_PROFILE_INHERIT_VALUE);
  assert.equal(seedingProfileSelectValue("profile-1"), "profile-1");
  assert.equal(
    seedingProfileSelectValueToId(SEEDING_PROFILE_INHERIT_VALUE),
    null,
  );
  assert.equal(seedingProfileSelectValueToId("profile-1"), "profile-1");
});

test("only torrent-capable indexers accept a seeding profile", () => {
  assert.equal(supportsSeedingProfileAssignment(["torrent"]), true);
  assert.equal(supportsSeedingProfileAssignment(["Torrent", "usenet"]), true);
  assert.equal(supportsSeedingProfileAssignment(["usenet"]), false);
  assert.equal(supportsSeedingProfileAssignment([]), false);
  assert.equal(supportsSeedingProfileAssignment(undefined), false);
});

test("table cells fall back to a dash when a goal defers to the client", () => {
  assert.equal(formatSeedingProfileRatio(null), "—");
  assert.equal(formatSeedingProfileRatio(1.5), "1.5");
  assert.equal(formatSeedingProfileSeedTime(null), "—");
  assert.equal(formatSeedingProfileSeedTime(60), "60m");
  assert.equal(
    formatSeasonPackSummary(
      { seasonPackMode: "INHERIT", seasonPackRatio: 2, seasonPackSeedTimeMinutes: 60 },
      "Inherit",
    ),
    "Inherit",
  );
  assert.equal(
    formatSeasonPackSummary(
      {
        seasonPackMode: "OVERRIDE",
        seasonPackRatio: 2,
        seasonPackSeedTimeMinutes: 60,
      },
      "Inherit",
    ),
    "2 / 60m",
  );
  assert.equal(
    formatSeasonPackSummary(
      {
        seasonPackMode: "OVERRIDE",
        seasonPackRatio: null,
        seasonPackSeedTimeMinutes: null,
      },
      "Inherit",
    ),
    "—",
  );
});

test("every seeding-profile string the UI renders has an English entry", () => {
  const requiredKeys = [
    "settings.seedingProfiles",
    "settings.seedingProfileExisting",
    "settings.seedingProfileNone",
    "settings.seedingProfileCreate",
    "settings.seedingProfileCreateNew",
    "settings.seedingProfileEdit",
    "settings.seedingProfileConfirmDiscardTitle",
    "settings.seedingProfileConfirmDiscardDescription",
    "settings.seedingProfileDeleteConfirm",
    "settings.seedingProfileNameLabel",
    "settings.seedingProfileNamePlaceholder",
    "settings.seedingProfileGoalPlaceholder",
    "settings.seedingProfileRatioLabel",
    "settings.seedingProfileRatioHelp",
    "settings.seedingProfileSeedTimeLabel",
    "settings.seedingProfileSeedTimeHelp",
    "settings.seedingProfileSeedTimeTransmissionHelp",
    "settings.seedingProfileSeasonPacksColumn",
    "settings.seedingProfileSeasonPackAdvanced",
    "settings.seedingProfileSeasonPackModeLabel",
    "settings.seedingProfileSeasonPackModeHelp",
    "settings.seedingProfileSeasonPackInherit",
    "settings.seedingProfileSeasonPackOverride",
    "settings.seedingProfileSeasonPackRatioLabel",
    "settings.seedingProfileSeasonPackSeedTimeLabel",
    "settings.seedingProfileSeasonPackGoalsHelp",
    "settings.seedingProfileHonorTrackerMinimumsLabel",
    "settings.seedingProfileHonorTrackerMinimumsHelp",
    "settings.seedingProfileGoalMetActionLabel",
    "settings.seedingProfileGoalMetActionHelp",
    "settings.seedingProfileGoalMetRemoveEntry",
    "settings.seedingProfileGoalMetStopSeeding",
    "settings.seedingProfileGoalMetKeep",
    "settings.seedingProfileNeverRemoveLabel",
    "settings.seedingProfileNeverRemoveHelp",
    "settings.seedingProfileNeverRemoveBadge",
    "settings.seedingProfileDefaultTitle",
    "settings.seedingProfileDefaultLabel",
    "settings.seedingProfileDefaultHelp",
    "settings.seedingProfileDefaultNone",
    "settings.seedingProfileDefaultBadge",
    "settings.seedingProfileMissing",
    "settings.seedingProfileSaved",
    "settings.seedingProfileSaveError",
    "settings.seedingProfileDeleted",
    "settings.seedingProfileDeleteError",
    "settings.seedingProfileDefaultSaved",
    "settings.seedingProfileInherit",
    "settings.seedingProfileRoutingInherit",
    "settings.seedingProfileColumn",
    "settings.seedingProfileIndexerLabel",
    "settings.seedingProfileRoutingLabel",
    "settings.seedingProfileNotApplicable",
    "status.indexerSeedingProfileSaving",
    "status.indexerSeedingProfileSaved",
  ];

  for (const key of requiredKeys) {
    assert.equal(typeof en[key], "string", `missing English string for ${key}`);
  }
});
