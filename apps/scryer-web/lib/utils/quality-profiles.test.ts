import test from "node:test";
import assert from "node:assert/strict";

import {
  commitQualityProfileDraftToEntries,
  hasDuplicateQualityProfileName,
} from "./quality-profile-draft-commit.ts";
import type { ParsedQualityProfileEntry, QualityProfileDraft } from "@/lib/types";

function makeEntry(id: string, name: string): ParsedQualityProfileEntry {
  return {
    id,
    name,
    criteria: {
      quality_tiers: ["2160P"],
      archival_quality: "2160P",
      allow_unknown_quality: false,
      source_allowlist: [],
      source_blocklist: [],
      video_codec_allowlist: [],
      video_codec_blocklist: [],
      audio_codec_allowlist: [],
      audio_codec_blocklist: [],
      dolby_vision_allowed: true,
      detected_hdr_allowed: true,
      prefer_remux: false,
      allow_bd_disk: true,
      allow_upgrades: true,
      scoring_overrides: {},
      cutoff_tier: null,
      min_score_to_grab: null,
    },
  };
}

function makeDraft(id: string, name: string): QualityProfileDraft {
  return {
    id,
    name,
    quality_tiers: ["2160P"],
    archival_quality: "2160P",
    allow_unknown_quality: false,
    source_allowlist: [],
    source_blocklist: [],
    video_codec_allowlist: [],
    video_codec_blocklist: [],
    audio_codec_allowlist: [],
    audio_codec_blocklist: [],
    dolby_vision_allowed: true,
    detected_hdr_allowed: true,
    prefer_remux: false,
    allow_bd_disk: true,
    allow_upgrades: true,
    scoring_overrides: {},
    cutoff_tier: "",
    min_score_to_grab: null,
  };
}

test("commitQualityProfileDraftToEntries preserves id when renaming an existing profile", () => {
  const entries = [makeEntry("default", "4K")];

  const committed = commitQualityProfileDraftToEntries(
    entries,
    makeDraft("default", "Cinema Prime"),
  );

  assert.equal(committed.catalogEntries.length, 1);
  assert.equal(committed.draftEntry.id, "default");
  assert.equal(committed.draftEntry.name, "Cinema Prime");
  assert.equal(committed.catalogEntries[0]?.id, "default");
  assert.equal(committed.catalogEntries[0]?.name, "Cinema Prime");
});

test("commitQualityProfileDraftToEntries appends a new profile for a create draft", () => {
  const entries = [makeEntry("default", "4K")];

  const committed = commitQualityProfileDraftToEntries(
    entries,
    makeDraft("", "Anime Max"),
  );

  assert.equal(committed.catalogEntries.length, 2);
  assert.equal(committed.draftEntry.id, "anime-max");
  assert.equal(committed.catalogEntries[1]?.id, "anime-max");
  assert.equal(committed.catalogEntries[1]?.name, "Anime Max");
});

test("hasDuplicateQualityProfileName ignores the current profile id during rename", () => {
  const entries = [
    makeEntry("default", "4K"),
    makeEntry("anime-max", "Anime Max"),
  ];

  assert.equal(hasDuplicateQualityProfileName(entries, "4K", "default"), false);
  assert.equal(hasDuplicateQualityProfileName(entries, " anime max ", "default"), true);
});
