/** Whether season packs reuse the profile's goals or override them. */
export type SeasonPackSeedMode = "INHERIT" | "OVERRIDE";

/** What happens to a torrent once its seeding goal is met. */
export type SeedGoalMetAction = "REMOVE_ENTRY" | "STOP_SEEDING" | "KEEP";

/** Whether Scryer keeps managing a torrent after it has been imported. */
export type PostImportTracking = "PARK" | "HAND_OFF";

/** A stored seeding profile as returned by `seedingProfiles`. */
export type SeedingProfileRecord = {
  id: string;
  name: string;
  /** Share ratio goal. null defers to the download client's own limits. */
  ratio: number | null;
  /** Seed time goal in minutes. null defers to the download client's own limits. */
  seedTimeMinutes: number | null;
  seasonPackMode: SeasonPackSeedMode;
  seasonPackRatio: number | null;
  seasonPackSeedTimeMinutes: number | null;
  honorTrackerMinimums: boolean;
  goalMetAction: SeedGoalMetAction;
  neverRemove: boolean;
  postImportTracking: PostImportTracking;
};

/**
 * Editor state. Goal fields are kept as raw strings so an empty field can mean
 * "defer to the client" and decimals survive round-tripping through the input.
 */
export type SeedingProfileDraft = {
  /** Empty for a profile that has not been created yet. */
  id: string;
  name: string;
  ratio: string;
  seedTimeMinutes: string;
  seasonPackMode: SeasonPackSeedMode;
  seasonPackRatio: string;
  seasonPackSeedTimeMinutes: string;
  honorTrackerMinimums: boolean;
  goalMetAction: SeedGoalMetAction;
  neverRemove: boolean;
  postImportTracking: PostImportTracking;
};

/** Minimal shape the assignment dropdowns need. */
export type SeedingProfileOption = {
  id: string;
  name: string;
};
