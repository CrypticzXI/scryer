import assert from "node:assert/strict";
import test from "node:test";

import {
  facetScopedMediaSettingsScopeId,
  updateFacetScopedStringArrayRecord,
  updateFacetScopedStringRecord,
} from "./media-settings-scope.ts";

test("saving an anime rename template updates the anime bucket", () => {
  const scopeId = facetScopedMediaSettingsScopeId({ scope: "anime" });
  const templates = {
    movie: "{title} ({year}) - {quality}.{ext}",
    series: "{title} - S{season:2}E{episode:2} - {quality}.{ext}",
    anime: "{title} - S{season_order:2}E{episode:2} ({absolute_episode}) - {quality}.{ext}",
  };

  const next = updateFacetScopedStringRecord(
    templates,
    scopeId,
    "{title} - {episode_title} - {source} - {group} - {quality}.{ext}",
  );

  assert.equal(next.anime, "{title} - {episode_title} - {source} - {group} - {quality}.{ext}");
  assert.equal(next.series, templates.series);
  assert.equal(next.movie, templates.movie);
});

test("reloading anime media settings restores values into anime scope rather than the current series bucket", () => {
  const scopeId = facetScopedMediaSettingsScopeId({ scope: "anime" });
  const renameTemplates = {
    movie: "{title} ({year}) - {quality}.{ext}",
    series: "SERIES CURRENT TEMPLATE",
    anime: "ANIME CURRENT TEMPLATE",
  };
  const audioLanguages = {
    movie: [] as string[],
    series: ["eng"],
    anime: ["jpn"],
  };

  const nextTemplates = updateFacetScopedStringRecord(
    renameTemplates,
    scopeId,
    "{title} - {episode_title} [{quality}].{ext}",
  );
  const nextAudioLanguages = updateFacetScopedStringArrayRecord(
    audioLanguages,
    scopeId,
    ["eng", "jpn"],
  );

  assert.equal(nextTemplates.series, "SERIES CURRENT TEMPLATE");
  assert.equal(nextTemplates.anime, "{title} - {episode_title} [{quality}].{ext}");
  assert.deepEqual(nextAudioLanguages.series, ["eng"]);
  assert.deepEqual(nextAudioLanguages.anime, ["eng", "jpn"]);
});
