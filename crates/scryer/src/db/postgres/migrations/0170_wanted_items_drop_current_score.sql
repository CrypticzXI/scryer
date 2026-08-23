-- Retire the per-scope score ledger.
--
-- `wanted_items.current_score` was never a reliable bar. Of its five lifecycle
-- states only one — after a successful import — held a landed score; after a
-- grab it held the score of an in-flight release, and after a rejected import it
-- held the score of a release that never landed at all, which is *below* the
-- incumbent and therefore lowered the bar. Grab compared against this number
-- while import compared against `media_files.acquisition_score`, and that
-- split is what let Scryer queue downloads it then refused to import.
--
-- The bar now comes from the library: the primary media file occupying the
-- scope, re-derived canonically at every comparison.
--
-- Three reads used the column as a *presence* flag rather than a score, all of
-- them meaning "did a scored import land here". `grabbed_release` already
-- carries that: it is cleared when an import lands and set while a grab is in
-- flight. Normalising it here lets those reads move over without a new column,
-- and without a lossy guess for rows written before that clearing existed.
UPDATE wanted_items
   SET grabbed_release = NULL
 WHERE status = 'completed'
   AND current_score IS NOT NULL;

ALTER TABLE wanted_items DROP COLUMN IF EXISTS current_score;
