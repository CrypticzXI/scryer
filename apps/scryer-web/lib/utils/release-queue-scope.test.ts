import assert from "node:assert/strict";
import test from "node:test";

import type { ReleaseQueueScope } from "@/lib/types/releases";
import { releaseSupportsAdditionalFileQueue } from "./release-queue-scope.ts";

test("additional-file queue eligibility uses the signed release queue scope", () => {
  assert.equal(
    releaseSupportsAdditionalFileQueue(
      { queueScope: { kind: "title", episodeIds: [] } },
      "movie",
    ),
    true,
  );
  assert.equal(
    releaseSupportsAdditionalFileQueue(
      { queueScope: { kind: "title", episodeIds: [] } },
      "series",
    ),
    false,
  );
  assert.equal(
    releaseSupportsAdditionalFileQueue(
      { queueScope: { kind: "title", episodeIds: [] } },
      "anime",
    ),
    false,
  );
  assert.equal(
    releaseSupportsAdditionalFileQueue(
      { queueScope: { kind: "episode", episodeId: "episode-1", episodeIds: [] } },
      "series",
    ),
    true,
  );
  assert.equal(
    releaseSupportsAdditionalFileQueue(
      {
        queueScope: {
          kind: "series_movie",
          seriesMovieLinkId: "series-movie-1",
          episodeIds: [],
        },
      },
      "anime",
    ),
    true,
  );

  const unsupportedScopes: ReleaseQueueScope[] = [
    { kind: "collection", collectionId: "season-1", episodeIds: [] },
    { kind: "episode_set", episodeIds: ["episode-1", "episode-2"] },
    { kind: "orphan", episodeIds: [] },
  ];
  for (const queueScope of unsupportedScopes) {
    assert.equal(releaseSupportsAdditionalFileQueue({ queueScope }, "movie"), false);
  }

  assert.equal(releaseSupportsAdditionalFileQueue({ queueScope: null }, "movie"), false);
});
