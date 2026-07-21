import assert from "node:assert/strict";
import test from "node:test";

import {
  buildOverviewDetailPath,
  resolveAppRoute,
  type ParsedAppRoute,
} from "./routing.ts";
import { isMediaSettingsSection } from "./routes.ts";

function canonical(path: string): ParsedAppRoute {
  const resolution = resolveAppRoute(path);
  assert.equal(resolution.kind, "canonical", path);
  if (resolution.kind !== "canonical") {
    throw new Error(`Expected canonical route for ${path}`);
  }
  return resolution.route;
}

function redirects(from: string, to: string): void {
  assert.deepEqual(resolveAppRoute(from), { kind: "redirect", to });
}

test("facet settings sections that consume media settings trigger loading", () => {
  for (const section of ["library", "general", "quality", "renaming", "routing"] as const) {
    assert.equal(isMediaSettingsSection(section), true, section);
  }

  for (const section of ["overview", "import"] as const) {
    assert.equal(isMediaSettingsSection(section), false, section);
  }
});

test("canonical route families resolve to typed application state", () => {
  for (const path of [
    "/movies",
    "/series",
    "/anime",
    "/discovery",
    "/requests",
    "/activity",
    "/activity/import",
    "/activity/history",
    "/calendar",
    "/automation/wanted/items",
    "/automation/wanted/cutoff-unmet",
    "/automation/wanted/pending",
    "/automation/wanted/history",
    "/automation/acquisition",
    "/automation/rules",
    "/automation/subtitles",
    "/automation/post-processing",
    "/integrations/indexers",
    "/integrations/download-clients",
    "/integrations/media-servers",
    "/integrations/notifications",
    "/settings/profile",
    "/settings/general",
    "/settings/quality-profiles",
    "/settings/delay-profiles",
    "/settings/plugins",
    "/system",
    "/system/jobs",
    "/system/recycle-bin",
    "/system/users",
    "/system/security",
    "/system/backup",
    "/logs",
    "/logs/audit",
  ]) {
    assert.equal(canonical(path).canonicalPath, path);
  }
});

test("media detail and settings routes reject ambiguous or extra segments", () => {
  assert.equal(canonical("/movies/sample-title").overviewTitleSlug, "sample-title");
  assert.equal(
    canonical("/series/library-a/sample-title").overviewLibrarySlug,
    "library-a",
  );
  assert.equal(
    canonical("/anime/settings/renaming").contentSettingsSection,
    "renaming",
  );
  assert.deepEqual(resolveAppRoute("/movies/settings/unknown"), {
    kind: "not-found",
  });
  assert.deepEqual(resolveAppRoute("/series/library-a/sample-title/extra"), {
    kind: "not-found",
  });
});

test("reserved title slugs use library-qualified paths", () => {
  const path = buildOverviewDetailPath("movies", "movies", "settings");
  assert.equal(path, "/movies/movies/settings");
  assert.equal(canonical(path).overviewTitleSlug, "settings");
});

test("0.16 route aliases redirect to canonical 0.17 paths", () => {
  for (const [from, to] of [
    ["/", "/movies"],
    ["/movies/overview", "/movies"],
    ["/series/settings", "/series/settings/library"],
    ["/series/media", "/series/settings/library"],
    ["/anime/requests", "/requests"],
    ["/wanted", "/automation/wanted/items"],
    ["/wanted/wanted-items", "/automation/wanted/items"],
    ["/wanted/wanted", "/automation/wanted/items"],
    ["/wanted/cutoff-unmet", "/automation/wanted/cutoff-unmet"],
    ["/wanted/cutoff", "/automation/wanted/cutoff-unmet"],
    ["/history", "/automation/wanted/history"],
    ["/settings/acquisition", "/automation/acquisition"],
    ["/settings/rules", "/automation/rules"],
    ["/settings/subtitles", "/automation/subtitles"],
    ["/settings/post-processing", "/automation/post-processing"],
    ["/settings/post-procesing", "/automation/post-processing"],
    ["/settings/indexers", "/integrations/indexers"],
    ["/settings/download-clients", "/integrations/download-clients"],
    ["/settings/downloadClients", "/integrations/download-clients"],
    ["/settings/media-servers", "/integrations/media-servers"],
    ["/settings/mediaServers", "/integrations/media-servers"],
    ["/settings/notifications", "/integrations/notifications"],
    ["/settings/users", "/system/users"],
    ["/settings/security", "/system/security"],
    ["/settings/qualityProfiles", "/settings/quality-profiles"],
    ["/settings/delayProfiles", "/settings/delay-profiles"],
    ["/settings/backup", "/system/backup"],
    ["/settings/backups", "/system/backup"],
    ["/settings/recycle-bin", "/system/recycle-bin"],
    ["/settings/recycleBin", "/system/recycle-bin"],
    ["/automation/post-procesing", "/automation/post-processing"],
    ["/system/overview", "/system"],
    ["/system/backups", "/system/backup"],
    ["/system/recycleBin", "/system/recycle-bin"],
    ["/system/logs", "/logs"],
    ["/system/audit", "/logs/audit"],
    ["/logs/logs", "/logs"],
    ["/logs/service", "/logs"],
    ["/logs/service-logs", "/logs"],
    ["/logs/audit-logs", "/logs/audit"],
  ] as const) {
    redirects(from, to);
  }
});

test("redirects preserve query parameters and hashes", () => {
  assert.deepEqual(resolveAppRoute(
    "/settings/recycleBin",
    "?library=library-a&id=title-id",
    "#items",
  ), {
    kind: "redirect",
    to: "/system/recycle-bin?library=library-a&id=title-id#items",
  });
});

test("legacy id-based media routes remain canonical until title lookup replaces them", () => {
  const resolution = resolveAppRoute("/movies", "?id=title-id&episodeId=episode-id");
  assert.equal(resolution.kind, "canonical");
});

test("unknown roots and invalid sections do not fall back to another page", () => {
  assert.deepEqual(resolveAppRoute("/unknown"), { kind: "not-found" });
  assert.deepEqual(resolveAppRoute("/system/unknown"), { kind: "not-found" });
  assert.deepEqual(resolveAppRoute("/automation/unknown"), { kind: "not-found" });
});
