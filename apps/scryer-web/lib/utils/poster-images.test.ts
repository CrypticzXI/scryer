import assert from "node:assert/strict";
import test from "node:test";

import {
  selectBackdropVariantUrl,
  selectMediaImageVariantUrl,
  selectPosterVariantUrl,
} from "./poster-images.ts";

test("rewriting an opaque media-image route preserves its token and base path", () => {
  assert.equal(
    selectMediaImageVariantUrl(
      "/scryer/images/media/opaque-token/w250?cache=1#poster",
      "w70",
    ),
    "/scryer/images/media/opaque-token/w70?cache=1#poster",
  );
});

test("rewriting an absolute opaque media-image route preserves its origin", () => {
  assert.equal(
    selectPosterVariantUrl(
      "https://scryer.example/base/images/media/opaque-token/original",
      "w250",
    ),
    "https://scryer.example/base/images/media/opaque-token/w250",
  );
});

test("rewriting a local poster variant drops the source variant version token", () => {
  assert.equal(
    selectPosterVariantUrl(
      "/images/titles/title-1/poster/w250?v=posterw250digest#poster",
      "w70",
    ),
    "/images/titles/title-1/poster/w70#poster",
  );
});

test("selecting the current local poster variant preserves its version token", () => {
  assert.equal(
    selectPosterVariantUrl("/images/titles/title-1/poster/w250?v=posterw250digest", "w250"),
    "/images/titles/title-1/poster/w250?v=posterw250digest",
  );
});

test("rewriting a legacy local w500 poster selects a supported variant", () => {
  assert.equal(
    selectPosterVariantUrl(
      "/images/titles/title-1/poster/w500?v=posterw500digest",
      "w250",
    ),
    "/images/titles/title-1/poster/w250",
  );
});

test("rewriting a TMDB poster original selects the closest card-sized variant", () => {
  assert.equal(
    selectPosterVariantUrl(
      "https://image.tmdb.org/t/p/original/abc123.jpg",
      "w250",
    ),
    "https://image.tmdb.org/t/p/w300/abc123.jpg",
  );
});

test("selecting the current TMDB poster variant preserves it", () => {
  assert.equal(
    selectPosterVariantUrl("https://image.tmdb.org/t/p/w300/abc123.jpg", "w250"),
    "https://image.tmdb.org/t/p/w300/abc123.jpg",
  );
});

test("rewriting a local fanart variant selects w1280 and drops stale version token", () => {
  assert.equal(
    selectBackdropVariantUrl(
      "/images/titles/title-1/fanart/w780?v=fanartw780digest#backdrop",
      "w1280",
    ),
    "/images/titles/title-1/fanart/w1280#backdrop",
  );
});

test("rewriting a TMDB image CDN backdrop selects w1280", () => {
  assert.equal(
    selectBackdropVariantUrl(
      "https://image.tmdb.org/t/p/w780/abc123.jpg",
      "w1280",
    ),
    "https://image.tmdb.org/t/p/w1280/abc123.jpg",
  );
});

test("rewriting a TMDB image CDN backdrop can select original", () => {
  assert.equal(
    selectBackdropVariantUrl(
      "https://image.tmdb.org/t/p/w1280/abc123.jpg",
      "original",
    ),
    "https://image.tmdb.org/t/p/original/abc123.jpg",
  );
});
