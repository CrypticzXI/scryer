import assert from "node:assert/strict";
import test from "node:test";

import {
  facetScopedMediaSettingsScopeId,
  updateFacetScopedStringArrayRecord,
  updateFacetScopedStringRecord,
} from "./media-settings-scope.ts";

test("saving an anime rename template updates the anime bucket", () => {
  const scopeId = facetScopedMediaSettingsScopeId({ scope: "ANIME" });
  const templates = {
    MOVIE: "{title} ({year}) - {quality}.{ext}",
    SERIES: "{title} - S{season:2}E{episode:2} - {quality}.{ext}",
    ANIME: "{title} - S{season_order:2}E{episode:2} ({absolute_episode}) - {quality}.{ext}",
  };

  const next = updateFacetScopedStringRecord(
    templates,
    scopeId,
    "{title} - {episode_title} - {source} - {group} - {quality}.{ext}",
  );

  assert.equal(next.ANIME, "{title} - {episode_title} - {source} - {group} - {quality}.{ext}");
  assert.equal(next.SERIES, templates.SERIES);
  assert.equal(next.MOVIE, templates.MOVIE);
});

test("reloading anime media settings restores values into anime scope rather than the current series bucket", () => {
  const scopeId = facetScopedMediaSettingsScopeId({ scope: "ANIME" });
  const renameTemplates = {
    MOVIE: "{title} ({year}) - {quality}.{ext}",
    SERIES: "SERIES CURRENT TEMPLATE",
    ANIME: "ANIME CURRENT TEMPLATE",
  };
  const audioLanguages = {
    MOVIE: [] as string[],
    SERIES: ["eng"],
    ANIME: ["jpn"],
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

  assert.equal(nextTemplates.SERIES, "SERIES CURRENT TEMPLATE");
  assert.equal(nextTemplates.ANIME, "{title} - {episode_title} [{quality}].{ext}");
  assert.deepEqual(nextAudioLanguages.SERIES, ["eng"]);
  assert.deepEqual(nextAudioLanguages.ANIME, ["eng", "jpn"]);
});
