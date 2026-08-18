import assert from "node:assert/strict";
import test from "node:test";

import { artworkFallbackStyle } from "./artwork-fallback.ts";

test("artwork fallback gradients are deterministic for a seed and tone", () => {
  const first = artworkFallbackStyle("Ember Anvil", "ANIME");
  const second = artworkFallbackStyle("Ember Anvil", "ANIME");

  assert.deepEqual(first, second);
  assert.match(String(first.backgroundImage), /radial-gradient/);
  assert.match(String(first.backgroundImage), /linear-gradient/);
});

test("artwork fallback gradients vary by seed and tone", () => {
  const anime = artworkFallbackStyle("Ember Anvil", "ANIME");
  const otherTitle = artworkFallbackStyle("Tide Chart", "ANIME");
  const series = artworkFallbackStyle("Ember Anvil", "SERIES");

  assert.notDeepEqual(anime, otherTitle);
  assert.notDeepEqual(anime, series);
});
