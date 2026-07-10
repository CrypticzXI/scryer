import assert from "node:assert/strict";
import test from "node:test";

import { Client, cacheExchange, fetchExchange } from "@urql/core";

// Exercises the freshness-first discovery contract at the urql layer: the
// document `cacheExchange` (not graphcache) sits ahead of `fetchExchange`, the
// client default stays `network-only`, and discovery call sites opt into
// `cache-first` for instant paint while a discovery event forces a
// `network-only` re-execution that refreshes the cached entry.
const DISCOVERY_QUERY = `query DiscoveryHome { discoveryHome { token } }`;

function createCountingClient() {
  let fetchCount = 0;
  const client = new Client({
    url: "http://localhost/graphql",
    requestPolicy: "network-only",
    exchanges: [cacheExchange, fetchExchange],
    fetch: (async () => {
      fetchCount += 1;
      return new Response(
        JSON.stringify({ data: { discoveryHome: { token: fetchCount } } }),
        { status: 200, headers: { "content-type": "application/json" } },
      );
    }) as typeof fetch,
  });
  return { client, fetchCount: () => fetchCount };
}

test("a second cache-first discovery read is served from the document cache", async () => {
  const { client, fetchCount } = createCountingClient();

  const first = await client
    .query(DISCOVERY_QUERY, {}, { requestPolicy: "cache-first" })
    .toPromise();
  assert.equal(first.error, undefined);
  assert.deepEqual(first.data, { discoveryHome: { token: 1 } });
  assert.equal(fetchCount(), 1);

  const second = await client
    .query(DISCOVERY_QUERY, {}, { requestPolicy: "cache-first" })
    .toPromise();
  assert.deepEqual(second.data, { discoveryHome: { token: 1 } });
  // No second network round-trip: the cached document answered the read.
  assert.equal(fetchCount(), 1);
});

test("a discovery event forces a network-only re-execution that refreshes the cache", async () => {
  const { client, fetchCount } = createCountingClient();

  await client
    .query(DISCOVERY_QUERY, {}, { requestPolicy: "cache-first" })
    .toPromise();
  assert.equal(fetchCount(), 1);

  // Mapping discovery_search_completed -> network-only re-execution bypasses the
  // cached entry and hits the server again.
  const refreshed = await client
    .query(DISCOVERY_QUERY, {}, { requestPolicy: "network-only" })
    .toPromise();
  assert.deepEqual(refreshed.data, { discoveryHome: { token: 2 } });
  assert.equal(fetchCount(), 2);

  // The refreshed value replaces the cache, so later cache-first reads serve it.
  const afterRefresh = await client
    .query(DISCOVERY_QUERY, {}, { requestPolicy: "cache-first" })
    .toPromise();
  assert.deepEqual(afterRefresh.data, { discoveryHome: { token: 2 } });
  assert.equal(fetchCount(), 2);
});
