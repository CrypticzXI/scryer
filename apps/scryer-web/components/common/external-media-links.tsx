import { useTranslate } from "@/lib/context/translate-context";

const imdbLogoUrl = `${import.meta.env.BASE_URL}media-sites/imdb.svg`;
const tvdbLogoUrl = `${import.meta.env.BASE_URL}media-sites/tvdb.svg`;
const tmdbLogoUrl = `${import.meta.env.BASE_URL}media-sites/tmdb.svg`;
const malLogoUrl = `${import.meta.env.BASE_URL}media-sites/mal.svg`;
const anilistLogoUrl = `${import.meta.env.BASE_URL}media-sites/anilist.svg`;
const anidbLogoUrl = `${import.meta.env.BASE_URL}media-sites/anidb.png`;

type ExternalMediaLinkButtonProps = {
  href: string | null;
  site: string;
  label?: string;
  logoSrc: string;
};

export function ExternalMediaLinkButton({
  href,
  site,
  label = site,
  logoSrc,
}: ExternalMediaLinkButtonProps) {
  const t = useTranslate();
  if (!href) {
    return null;
  }

  return (
    <a
      href={href}
      target="_blank"
      rel="noreferrer"
      className="inline-flex h-12 items-center gap-2 rounded-md border border-border bg-card/45 px-3 py-2 text-base hover:bg-muted"
      aria-label={t("external.openOn", { site })}
    >
      <img src={logoSrc} alt={site} className="h-8 w-8" />
      <span className="text-muted-foreground">{label}</span>
    </a>
  );
}

export function ImdbExternalLink({ imdbId }: { imdbId: string | null | undefined }) {
  return (
    <ExternalMediaLinkButton
      href={imdbTitleUrl(imdbId)}
      site="IMDb"
      logoSrc={imdbLogoUrl}
    />
  );
}

export function TvdbMovieExternalLink({
  slug,
  tvdbId,
}: {
  slug?: string | null;
  tvdbId: string | null | undefined;
}) {
  return (
    <ExternalMediaLinkButton
      href={tvdbMovieUrl(tvdbId, slug)}
      site="TVDB"
      logoSrc={tvdbLogoUrl}
    />
  );
}

export function TvdbSeriesExternalLink({
  slug,
  tvdbId,
}: {
  slug?: string | null;
  tvdbId?: string | null;
}) {
  return (
    <ExternalMediaLinkButton
      href={tvdbSeriesUrl(slug, tvdbId)}
      site="TVDB"
      logoSrc={tvdbLogoUrl}
    />
  );
}

export function TmdbExternalLink({
  mediaType,
  tmdbId,
}: {
  mediaType: "movie" | "tv";
  tmdbId: string | null | undefined;
}) {
  return (
    <ExternalMediaLinkButton
      href={tmdbUrl(tmdbId, mediaType)}
      site="TMDB"
      logoSrc={tmdbLogoUrl}
    />
  );
}

export function MalExternalLink({ malId }: { malId: string | null | undefined }) {
  return (
    <ExternalMediaLinkButton
      href={numericProviderUrl("https://myanimelist.net/anime", malId)}
      site="MyAnimeList"
      label="MAL"
      logoSrc={malLogoUrl}
    />
  );
}

export function AnilistExternalLink({ anilistId }: { anilistId: string | null | undefined }) {
  return (
    <ExternalMediaLinkButton
      href={numericProviderUrl("https://anilist.co/anime", anilistId)}
      site="AniList"
      logoSrc={anilistLogoUrl}
    />
  );
}

export function AnidbExternalLink({ anidbId }: { anidbId: string | null | undefined }) {
  return (
    <ExternalMediaLinkButton
      href={numericProviderUrl("https://anidb.net/anime", anidbId)}
      site="AniDB"
      logoSrc={anidbLogoUrl}
    />
  );
}

function imdbTitleUrl(imdbId: string | null | undefined) {
  const trimmed = imdbId?.trim();
  if (!trimmed) {
    return null;
  }
  if (trimmed.startsWith("tt")) {
    return `https://www.imdb.com/title/${trimmed}`;
  }
  return `https://www.imdb.com/find?q=${encodeURIComponent(trimmed)}&s=tt`;
}

function tvdbMovieUrl(tvdbIdValue: string | null | undefined, slugValue?: string | null) {
  const tvdbId = tvdbIdValue?.trim();
  if (!tvdbId) {
    return null;
  }
  const slug = slugValue?.trim();
  const base = "https://www.thetvdb.com";
  if (slug) {
    return `${base}/movies/${encodeURIComponent(slug)}`;
  }
  return `${base}/?id=${encodeURIComponent(tvdbId)}`;
}

function tvdbSeriesUrl(slugValue?: string | null, tvdbIdValue?: string | null) {
  const slug = slugValue?.trim();
  if (slug) {
    return `https://thetvdb.com/series/${slug}`;
  }
  const tvdbId = tvdbIdValue?.trim();
  if (!tvdbId) {
    return null;
  }
  return `https://thetvdb.com/?id=${encodeURIComponent(tvdbId)}`;
}

function tmdbUrl(tmdbId: string | null | undefined, mediaType: "movie" | "tv") {
  const trimmed = tmdbId?.trim();
  if (!trimmed) {
    return null;
  }
  return `https://www.themoviedb.org/${mediaType}/${encodeURIComponent(trimmed)}`;
}

function numericProviderUrl(baseUrl: string, providerId: string | null | undefined) {
  const trimmed = providerId?.trim();
  if (!trimmed) {
    return null;
  }
  return `${baseUrl}/${encodeURIComponent(trimmed)}`;
}
