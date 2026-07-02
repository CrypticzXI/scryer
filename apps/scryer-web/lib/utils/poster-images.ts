export type LocalPosterVariant = "original" | "w500" | "w250" | "w70";
export type LocalBackdropVariant = "original" | "w1280";

const LOCAL_TITLE_POSTER_PATH_RE = /^(.*\/images\/titles\/[^/]+\/poster\/)(original|w500|w250|w70)$/;
const LOCAL_TITLE_BACKDROP_PATH_RE = /^(.*\/images\/titles\/[^/]+\/fanart\/)(original|w\d+)$/;
const TMDB_IMAGE_PATH_RE = /^(\/t\/p\/)(original|w\d+)(\/.+)$/;

export function selectPosterVariantUrl(
  posterUrl: string | null | undefined,
  desiredVariant: LocalPosterVariant,
): string | null | undefined {
  if (!posterUrl) {
    return posterUrl;
  }

  try {
    const parsed = new URL(posterUrl, "http://scryer.local");
    const match = parsed.pathname.match(LOCAL_TITLE_POSTER_PATH_RE);
    if (!match) {
      return posterUrl;
    }

    const [, prefix, currentVariant] = match;
    if (currentVariant === "original" || currentVariant === desiredVariant) {
      return posterUrl;
    }

    parsed.pathname = `${prefix}${desiredVariant}`;
    parsed.searchParams.delete("v");
    return isRelativeUrl(posterUrl)
      ? `${parsed.pathname}${parsed.search}${parsed.hash}`
      : parsed.toString();
  } catch {
    return posterUrl;
  }
}

export function selectBackdropVariantUrl(
  backdropUrl: string | null | undefined,
  desiredVariant: LocalBackdropVariant,
): string | null | undefined {
  if (!backdropUrl) {
    return backdropUrl;
  }

  try {
    const parsed = new URL(backdropUrl, "http://scryer.local");
    const localMatch = parsed.pathname.match(LOCAL_TITLE_BACKDROP_PATH_RE);
    if (localMatch) {
      const [, prefix, currentVariant] = localMatch;
      if (currentVariant === desiredVariant) {
        return backdropUrl;
      }

      parsed.pathname = `${prefix}${desiredVariant}`;
      parsed.searchParams.delete("v");
      return isRelativeUrl(backdropUrl)
        ? `${parsed.pathname}${parsed.search}${parsed.hash}`
        : parsed.toString();
    }

    if (parsed.hostname === "image.tmdb.org") {
      const tmdbMatch = parsed.pathname.match(TMDB_IMAGE_PATH_RE);
      if (tmdbMatch) {
        const [, prefix, currentVariant, suffix] = tmdbMatch;
        if (currentVariant === desiredVariant) {
          return backdropUrl;
        }

        parsed.pathname = `${prefix}${desiredVariant}${suffix}`;
        return parsed.toString();
      }
    }

    return backdropUrl;
  } catch {
    return backdropUrl;
  }
}

function isRelativeUrl(url: string): boolean {
  return !/^[a-zA-Z][a-zA-Z\d+\-.]*:/.test(url) && !url.startsWith("//");
}
