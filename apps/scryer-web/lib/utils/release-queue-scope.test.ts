import assert from "node:assert/strict";
import test from "node:test";

import type { ReleaseQueueScope } from "@/lib/types/releases";
import { releaseSupportsAdditionalFileQueue } from "./release-queue-scope.ts";

test("additional-file queue eligibility uses the signed release queue scope", () => {
  assert.equal(
    releaseSupportsAdditionalFileQueue(
      { queueScope: { __typename: "TitleScopePayload", wholeTitle: true } },
      "movie",
    ),
    true,
  );
  assert.equal(
    releaseSupportsAdditionalFileQueue(
      { queueScope: { __typename: "TitleScopePayload", wholeTitle: true } },
      "series",
    ),
    false,
  );
  assert.equal(
    releaseSupportsAdditionalFileQueue(
      { queueScope: { __typename: "TitleScopePayload", wholeTitle: true } },
      "anime",
    ),
    false,
  );
  assert.equal(
    releaseSupportsAdditionalFileQueue(
      { queueScope: { __typename: "EpisodeScopePayload", episodeId: "episode-1" } },
      "series",
    ),
    true,
  );
  assert.equal(
    releaseSupportsAdditionalFileQueue(
      {
        queueScope: {
          __typename: "SeriesMovieScopePayload",
          seriesMovieLinkId: "series-movie-1",
        },
      },
      "anime",
    ),
    true,
  );

  const unsupportedScopes: ReleaseQueueScope[] = [
    { __typename: "CollectionScopePayload", collectionId: "season-1" },
    { __typename: "EpisodeSetScopePayload", episodeIds: ["episode-1", "episode-2"] },
    { __typename: "OrphanScopePayload", orphaned: true },
  ];
  for (const queueScope of unsupportedScopes) {
    assert.equal(releaseSupportsAdditionalFileQueue({ queueScope }, "movie"), false);
  }

  assert.equal(releaseSupportsAdditionalFileQueue({ queueScope: null }, "movie"), false);
});
