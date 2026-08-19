import assert from "node:assert/strict";
import test, { after, before } from "node:test";
import { fileURLToPath } from "node:url";
import type { DownloadQueueItem } from "../types/download-queue.ts";
import { createServer, type ViteDevServer } from "vite";

const WEB_ROOT = fileURLToPath(new URL("../..", import.meta.url));

type QueueRowResult = {
  failureReason: string;
  hasStatusDetails: boolean;
};

type DeriveQueueRowPresentation = (
  item: DownloadQueueItem,
  translate: (key: string) => string,
) => QueueRowResult;

let server: ViteDevServer;
let deriveQueueRowPresentation: DeriveQueueRowPresentation;

before(async () => {
  server = await createServer({
    root: WEB_ROOT,
    server: { middlewareMode: true },
    appType: "custom",
    logLevel: "silent",
  });
  const module = await server.ssrLoadModule("/lib/utils/activity-utils.ts");
  deriveQueueRowPresentation =
    module.deriveQueueRowPresentation as DeriveQueueRowPresentation;
});

after(async () => {
  await server.close();
});

const translations: Record<string, string> = {
  "queue.blockReasonFallbackUnassigned":
    "Automatic import could not identify a library title. Assign a title to continue.",
  "queue.blockReasonFallbackEpisodic":
    "Automatic import could not determine a unique season and episode mapping. Open Manual Import and assign the correct season and episode.",
  "queue.blockReasonFallbackReview":
    "Automatic import needs operator review. Open Manual Import and confirm the file mapping to continue.",
};

const translate = (key: string) => translations[key] ?? key;

function blockedItem(overrides: Partial<DownloadQueueItem> = {}): DownloadQueueItem {
  return {
    id: "queue-1",
    titleId: null,
    episodeId: null,
    titleName:
      "[Erai-raws].Yuki-sama.Kagami.no.Toki.Desu-09.[1080p][Multiple.Subtitle][AA7AC7E5]",
    facet: "ANIME",
    isScryerOrigin: true,
    sourceProvider: null,
    clientId: "client-1",
    clientName: "Weaver",
    clientType: "weaver",
    state: "COMPLETED",
    displayState: "IMPORT_BLOCKED",
    progressPercent: 100,
    importTransferPhase: null,
    importTransferBytes: null,
    importTransferTotalBytes: null,
    importTransferStartedAt: null,
    importTransferUpdatedAt: null,
    sizeBytes: null,
    remainingSeconds: null,
    queuedAt: null,
    lastUpdatedAt: null,
    attentionRequired: true,
    attentionReason: null,
    downloadClientItemId: "download-1",
    downloadId: "scryer-download:queue-1",
    importStatus: null,
    importErrorCode: null,
    importErrorMessage: null,
    importedAt: null,
    deleteStatus: null,
    deleteErrorMessage: null,
    trackedState: "IMPORT_BLOCKED",
    trackedStatus: "WARNING",
    trackedStatusMessages: [],
    trackedMatchType: "UNMATCHED",
    queueScope: null,
    ...overrides,
  };
}

test("blocked queue rows explain that an unassigned title needs assignment", () => {
  const row = deriveQueueRowPresentation(blockedItem(), translate);

  assert.equal(
    row.failureReason,
    translations["queue.blockReasonFallbackUnassigned"],
  );
  assert.equal(row.hasStatusDetails, true);
  assert.notEqual(row.failureReason, "—");
});

test("blocked episodic rows direct the operator to season and episode mapping", () => {
  const row = deriveQueueRowPresentation(
    blockedItem({ titleId: "title-1", trackedMatchType: "SUBMISSION" }),
    translate,
  );

  assert.equal(
    row.failureReason,
    translations["queue.blockReasonFallbackEpisodic"],
  );
});

test("blocked movie rows direct the operator to review the file mapping", () => {
  const row = deriveQueueRowPresentation(
    blockedItem({ titleId: "title-1", facet: "MOVIE" }),
    translate,
  );

  assert.equal(
    row.failureReason,
    translations["queue.blockReasonFallbackReview"],
  );
});

test("backend import-block detail takes precedence over frontend fallback copy", () => {
  const backendReason =
    "Automatic import could not choose a season for episode 9 because the downloaded filename is obfuscated.";
  const row = deriveQueueRowPresentation(
    blockedItem({
      titleId: "title-1",
      trackedStatusMessages: [backendReason],
    }),
    translate,
  );

  assert.equal(row.failureReason, backendReason);
});
