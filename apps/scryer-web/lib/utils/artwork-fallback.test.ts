import assert from "node:assert/strict";
import test from "node:test";

import { artworkFallbackStyle } from "./artwork-fallback.ts";

test("artwork fallback gradients are deterministic for a seed and tone", () => {
  const first = artworkFallbackStyle("Tougen Anki", "ANIME");
  const second = artworkFallbackStyle("Tougen Anki", "ANIME");

  assert.deepEqual(first, second);
  assert.match(String(first.backgroundImage), /radial-gradient/);
  assert.match(String(first.backgroundImage), /linear-gradient/);
});

test("artwork fallback gradients vary by seed and tone", () => {
  const anime = artworkFallbackStyle("Tougen Anki", "ANIME");
  const otherTitle = artworkFallbackStyle("One Piece", "ANIME");
  const series = artworkFallbackStyle("Tougen Anki", "SERIES");

  assert.notDeepEqual(anime, otherTitle);
  assert.notDeepEqual(anime, series);
});
