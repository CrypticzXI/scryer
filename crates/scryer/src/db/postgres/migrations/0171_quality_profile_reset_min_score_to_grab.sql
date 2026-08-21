-- Reset the quality-profile score floor.
--
-- `min_score_to_grab` is Sonarr's `MinFormatScore`: an absolute floor a release
-- must clear before it may be grabbed at all. Every value a user set for it was
-- calibrated against the old, tier-inclusive scale, where each listed quality
-- tier contributed 3200/900/300 points and an ordinary release therefore scored
-- in the thousands.
--
-- The tier has left the score. It is now compared before the score, in the
-- admission gate, and the number that remains is a *within-tier* preference
-- quantity: ordinary releases land roughly between -700 and +1500, and a
-- perfectly good 2160p WEB-DL can score -60. A floor of, say, 2000 that used to
-- exclude the bottom of the barrel now excludes literally everything, and it
-- does it as a hard block with no error anywhere: the profile simply stops
-- acquiring, silently, forever.
--
-- Nulling the field is the only safe migration. There is no rescaling function
-- that could be right, because the old number mixed a tier bonus (now gone)
-- with preference terms (now the whole score), and how much of it was which
-- depends on the profile's tier list. Built-in profiles and TRaSH packs leave
-- the field unset, so this only touches profiles a user configured by hand; the
-- field keeps its Sonarr meaning for any value set from now on, against the new
-- scale.
--
-- The value lives inside the `scoring_config` JSON blob rather than in a column,
-- so the key is removed rather than set to null. `ScoringConfig` deserializes it
-- with `#[serde(default)]`, so an absent key reads back as `None`.
UPDATE quality_profiles
   SET scoring_config = scoring_config - 'min_score_to_grab'
 WHERE jsonb_exists(scoring_config, 'min_score_to_grab')
   AND jsonb_typeof(scoring_config -> 'min_score_to_grab') <> 'null';
