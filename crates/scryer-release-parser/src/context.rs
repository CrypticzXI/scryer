use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// Required media facet for a target-aware parse.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextFacetHint {
    Movie,
    Series,
    Anime,
    #[default]
    Unknown,
}

/// Canonical title metadata for target-aware parsing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextTitle {
    pub name: String,
}

/// Title alias metadata for target-aware parsing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextAlias {
    pub name: String,
}

/// Episode metadata for target-aware parsing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextEpisode {
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub absolute_number: Option<u32>,
    pub air_date: Option<NaiveDate>,
    pub title: Option<String>,
    pub title_aliases: Vec<String>,
}

/// Required target metadata supplied by the caller for authoritative parsing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseParseContext {
    pub facet_hint: ContextFacetHint,
    pub title: ContextTitle,
    pub aliases: Vec<ContextAlias>,
    pub known_years: Vec<i32>,
    pub imdb_ids: Vec<String>,
    pub episodes: Vec<ContextEpisode>,
}
