import assert from "node:assert/strict";
import test from "node:test";

import {
  titleCastCreditCharacter,
  titleCastCreditEpisodeCount,
  titleCastCreditKey,
  titleCastCredits,
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
