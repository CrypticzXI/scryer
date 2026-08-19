import assert from "node:assert/strict";
import test from "node:test";

import {
  TITLE_CAST_RAIL_DISPLAY_LIMIT,
  titleCastCreditCharacter,
  titleCastCreditEpisodeCount,
  titleCastCreditKey,
  titleCastCredits,
  titleCastDubCredits,
  titleCastDubLanguageLabel,
  titleCastDubLanguages,
  titleCastOriginalCredits,
  titleCastPreferredDubLanguage,
} from "./title-cast.ts";

function credit(overrides: Record<string, unknown> = {}) {
  return {
    kind: "actor",
    personName: "Lead Actor",
    personOriginalName: "",
    personImageUrl: null,
    character: "",
    language: "eng",
    billingOrder: 0,
    episodeCount: null,
    ...overrides,
  };
}

test("cast credits keep the server's order", () => {
  // The server already filtered by kind, sorted by billing rank, and truncated;
  // re-sorting here would fight it.
  const ordered = titleCastCredits([
    credit({ personName: "Second", billingOrder: 3 }),
    credit({ personName: "First", billingOrder: 9 }),
  ]);

  assert.deepEqual(
    ordered.map((entry) => entry.personName),
    ["Second", "First"],
  );
});

test("credits with no renderable name are dropped", () => {
  const visible = titleCastCredits([
    credit({ personName: "" }),
    credit({ personName: "   " }),
    credit({ personName: "Named" }),
  ]);

  assert.deepEqual(
    visible.map((entry) => entry.personName),
    ["Named"],
  );
});

test("missing credits render an empty rail rather than throwing", () => {
  assert.deepEqual(titleCastCredits(undefined), []);
  assert.deepEqual(titleCastCredits(null), []);
});

test("cast card keys stay unique when a provider bills two people the same", () => {
  const duplicates = [
    credit({ personName: "Same Name", billingOrder: 1 }),
    credit({ personName: "Same Name", billingOrder: 1 }),
  ];
  const keys = duplicates.map((entry, index) =>
    titleCastCreditKey(entry, index),
  );

  assert.equal(new Set(keys).size, 2);
});

test("episode counts render only when the provider actually counted", () => {
  assert.equal(titleCastCreditEpisodeCount(credit({ episodeCount: 12 })), 12);
  // Movies have no episode count, and zero or negative counts are noise.
  assert.equal(titleCastCreditEpisodeCount(credit({ episodeCount: null })), null);
  assert.equal(
    titleCastCreditEpisodeCount(credit({ episodeCount: undefined })),
    null,
  );
  assert.equal(titleCastCreditEpisodeCount(credit({ episodeCount: 0 })), null);
  assert.equal(titleCastCreditEpisodeCount(credit({ episodeCount: -3 })), null);
});

test("character sublines collapse the provider's empty strings to null", () => {
  assert.equal(titleCastCreditCharacter(credit({ character: "Hero" })), "Hero");
  assert.equal(titleCastCreditCharacter(credit({ character: "" })), null);
  assert.equal(titleCastCreditCharacter(credit({ character: "  " })), null);
  assert.equal(titleCastCreditCharacter(credit({ character: null })), null);
});

test("the main rail carries the original cast, never the dub", () => {
  const credits = [
    credit({ kind: "voice_actor", personName: "Seiyuu", language: "ja" }),
    credit({ kind: "voice_actor", personName: "Dub Actor", language: "en" }),
  ];

  assert.deepEqual(
    titleCastOriginalCredits(credits).map((entry) => entry.personName),
    ["Seiyuu"],
  );
  assert.deepEqual(
    titleCastDubCredits(credits).map((entry) => entry.personName),
    ["Dub Actor"],
  );
});

test("live-action actor rows stay on the main rail and produce no dub rail", () => {
  // TMDB actor rows carry no meaningful language; they must not be mistaken for
  // a dub just because the language is not "ja".
  const credits = [
    credit({ kind: "actor", personName: "Screen Actor", language: "eng" }),
    credit({ kind: "actor", personName: "Unlabelled", language: "" }),
  ];

  assert.deepEqual(
    titleCastOriginalCredits(credits).map((entry) => entry.personName),
    ["Screen Actor", "Unlabelled"],
  );
  assert.deepEqual(titleCastDubCredits(credits), []);
});

test("each rail caps independently at the display limit", () => {
  const many = Array.from({ length: 40 }, (_, index) => [
    credit({
      kind: "voice_actor",
      personName: `JP ${index}`,
      language: "ja",
      billingOrder: index,
    }),
    credit({
      kind: "voice_actor",
      personName: `EN ${index}`,
      language: "en",
      billingOrder: index,
    }),
  ]).flat();

  assert.equal(
    titleCastOriginalCredits(many).length,
    TITLE_CAST_RAIL_DISPLAY_LIMIT,
  );
  assert.equal(titleCastDubCredits(many).length, TITLE_CAST_RAIL_DISPLAY_LIMIT);
  // Capping happens after the split, so a dub-heavy response cannot crowd the
  // original cast out of its own rail.
  assert.equal(titleCastOriginalCredits(many)[0].personName, "JP 0");
  assert.equal(titleCastDubCredits(many)[0].personName, "EN 0");
});

test("dub languages are discovered from the credits themselves", () => {
  // Only en flows today; de/es appear on their own once SMG widens VA_LANGUAGES.
  const credits = [
    credit({ kind: "voice_actor", personName: "Seiyuu", language: "ja" }),
    credit({ kind: "voice_actor", personName: "EN", language: "en" }),
    credit({ kind: "voice_actor", personName: "DE", language: "de" }),
    credit({ kind: "voice_actor", personName: "DE 2", language: "de" }),
    credit({ kind: "actor", personName: "Screen", language: "eng" }),
  ];

  // Japanese is the main rail, and live-action actors are never dub options.
  assert.deepEqual(titleCastDubLanguages(credits), ["de", "en"]);
  assert.deepEqual(titleCastDubLanguages([]), []);
});

test("the dub rail filters to the selected language", () => {
  const credits = [
    credit({ kind: "voice_actor", personName: "EN", language: "en" }),
    credit({ kind: "voice_actor", personName: "DE", language: "de" }),
  ];

  assert.deepEqual(
    titleCastDubCredits(credits, "de").map((entry) => entry.personName),
    ["DE"],
  );
});

test("the dub picker defaults to English and survives a language going away", () => {
  assert.equal(titleCastPreferredDubLanguage(["de", "en"], null), "en");
  assert.equal(titleCastPreferredDubLanguage(["de", "en"], "de"), "de");
  // A remembered pick that this title has no credits for falls back rather
  // than selecting an option the picker does not list.
  assert.equal(titleCastPreferredDubLanguage(["en"], "de"), "en");
  assert.equal(titleCastPreferredDubLanguage(["fr"], "de"), "fr");
  assert.equal(titleCastPreferredDubLanguage([], "en"), null);
});

test("dub language labels are human readable with a raw-code fallback", () => {
  assert.equal(titleCastDubLanguageLabel("de", "en"), "German");
  assert.equal(titleCastDubLanguageLabel("en", "en"), "English");
  assert.equal(titleCastDubLanguageLabel("zzzz", "en"), "zzzz");
});
