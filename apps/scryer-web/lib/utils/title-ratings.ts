export type TitleExternalRating = {
  source: string;
  value: number | null;
  score: number | null;
  normalized: number;
  votes: number | null;
  url: string;
};

export type RatingSourceInfo = {
  label: string;
  logoSrc: string | null;
  format: "default" | "percent" | "hundred";
};

export function normalizedRatingSource(source: string): string {
  return source.trim().toLowerCase().replace(/[\s_-]+/g, "");
}

function fallbackSourceLabel(source: string): string {
  return source
    .trim()
    .split(/[\s_-]+/)
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

export function ratingSourceInfo(source: string): RatingSourceInfo {
  const normalized = normalizedRatingSource(source);
  switch (normalized) {
    case "imdb":
      return { label: "IMDb", logoSrc: "/rating-sources/imdb.svg", format: "default" };
    case "rottentomatoes":
    case "tomatoes":
      return {
        label: "Rotten Tomatoes",
        logoSrc: "/rating-sources/rotten-tomatoes.svg",
        format: "percent",
      };
    case "audience":
      return {
        label: "RT Audience",
        logoSrc: "/rating-sources/rotten-tomatoes.svg",
        format: "percent",
      };
    case "popcorn":
      return {
        label: "Popcornmeter",
        logoSrc: "/rating-sources/rotten-tomatoes.svg",
        format: "percent",
      };
    case "metacritic":
      return {
        label: "Metacritic",
        logoSrc: "/rating-sources/metacritic.svg",
        format: "hundred",
      };
    case "tmdb":
      return { label: "TMDb", logoSrc: "/rating-sources/tmdb.svg", format: "default" };
    case "trakt":
      return { label: "Trakt", logoSrc: "/rating-sources/trakt.svg", format: "default" };
    case "letterboxd":
      return {
        label: "Letterboxd",
        logoSrc: "/rating-sources/letterboxd.svg",
        format: "default",
      };
    case "mdblist":
      return { label: "MDBList", logoSrc: "/rating-sources/mdblist.avif", format: "default" };
    case "mal":
      return { label: "MAL", logoSrc: "/rating-sources/mal.svg", format: "default" };
    default:
      return { label: fallbackSourceLabel(source), logoSrc: null, format: "default" };
  }
}

export function compactRatingNumber(value: number): string {
  if (Number.isInteger(value)) {
    return value.toString();
  }
  return value.toFixed(1).replace(/\.0$/, "");
}

function scoreOutOfHundred(rating: TitleExternalRating): number {
  const score = rating.score ?? rating.value;
  if (score != null) {
    if (score <= 1) {
      return score * 100;
    }
    if (score <= 10) {
      return score * 10;
    }
    return score;
  }
  return rating.normalized * 10;
}

export function ratingValueLabel(
  rating: TitleExternalRating,
  source = ratingSourceInfo(rating.source),
): string {
  if (source.format === "percent") {
    return `${Math.round(scoreOutOfHundred(rating))}%`;
  }
  if (source.format === "hundred") {
    return compactRatingNumber(scoreOutOfHundred(rating));
  }
  const value = rating.value ?? rating.normalized;
  return compactRatingNumber(value);
}
