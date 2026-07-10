-- RFC 121 SW4.1: the discovery_raw_pages payload store was write-only in
-- production (no SELECT ever read raw_payload; only a data-agnostic backup
-- catalog entry and a test COUNT referenced it). Pages are still fetched and
-- parsed during ingest, they are simply no longer persisted raw. Drop the table
-- and its index so the projection layer stops accumulating uncompressed blobs.
DROP INDEX IF EXISTS idx_discovery_raw_pages_run;
DROP TABLE IF EXISTS discovery_raw_pages;
