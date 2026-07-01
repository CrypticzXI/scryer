import { Clock3, ExternalLink, Send, Sparkles } from "lucide-react";

import { TitlePoster } from "@/components/title-poster";
import {
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import type { Facet } from "@/lib/types";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";
import {
  ratingSourceInfo,
  ratingValueLabel,
  type TitleExternalRating,
} from "@/lib/utils/title-ratings";

type CatalogActionDialogSummaryProps = {
  result: MetadataTvdbSearchItem;
  facet: Facet;
  mode: "add" | "request";
};

type ExternalSiteLink = {
  id: string;
  label: string;
  href: string;
  logoSrc: string | null;
};

const MEDIA_SITE_LOGOS: Record<string, string> = {
  imdb: "/media-sites/imdb.svg",
  mal: "/media-sites/mal.svg",
  tmdb: "/media-sites/tmdb.svg",
  trakt: "/rating-sources/trakt.svg",
};

function normalizeSource(value: string) {
  return value.trim().toLowerCase().replace(/[\s_.-]+/g, "");
}

function externalIdMap(result: MetadataTvdbSearchItem) {
  const ids = new Map<string, string>();
  for (const externalId of result.externalIds ?? []) {
    const source = normalizeSource(externalId.source);
    const value = externalId.value.trim();
    if (source && value && !ids.has(source)) {
      ids.set(source, value);
    }
  }
  if (result.imdbId?.trim() && !ids.has("imdb")) {
    ids.set("imdb", result.imdbId.trim());
  }
  if (result.tvdbId.trim() && !ids.has("tvdb")) {
    ids.set("tvdb", result.tvdbId.trim());
  }
  return ids;
}

function tmdbPathForFacet(facet: Facet, type: string | null | undefined) {
  const normalizedType = type?.trim().toLowerCase();
  if (normalizedType === "movie" || facet === "movie") {
    return "movie";
  }
  return "tv";
}

function externalLinks(result: MetadataTvdbSearchItem, facet: Facet): ExternalSiteLink[] {
  const ids = externalIdMap(result);
  const links: ExternalSiteLink[] = [];
  const seen = new Set<string>();
  const push = (link: ExternalSiteLink | null) => {
    if (!link || seen.has(link.id)) {
      return;
    }
    seen.add(link.id);
    links.push(link);
  };

  const imdbId = ids.get("imdb");
  push(
    imdbId
      ? {
          id: "imdb",
          label: "IMDb",
          href: `https://www.imdb.com/title/${encodeURIComponent(imdbId)}/`,
          logoSrc: MEDIA_SITE_LOGOS.imdb,
        }
      : null,
  );

  const tmdbId = ids.get("tmdb");
  push(
    tmdbId
      ? {
          id: "tmdb",
          label: "TMDB",
          href: `https://www.themoviedb.org/${tmdbPathForFacet(
            facet,
            result.type,
          )}/${encodeURIComponent(tmdbId)}`,
          logoSrc: MEDIA_SITE_LOGOS.tmdb,
        }
      : null,
  );

  const malId = ids.get("mal") ?? ids.get("myanimelist");
  push(
    malId
      ? {
          id: "mal",
          label: "MAL",
          href: `https://myanimelist.net/anime/${encodeURIComponent(malId)}`,
          logoSrc: MEDIA_SITE_LOGOS.mal,
        }
      : null,
  );

  const query = [result.name, result.year].filter(Boolean).join(" ");
  push(
    query
      ? {
          id: "trakt",
          label: "Trakt",
          href: `https://trakt.tv/search?query=${encodeURIComponent(query)}`,
          logoSrc: MEDIA_SITE_LOGOS.trakt,
        }
      : null,
  );

  return links.slice(0, 4);
}

function ratingEntries(result: MetadataTvdbSearchItem): TitleExternalRating[] {
  const externalRatings = result.externalRatings ?? [];
  if (externalRatings.length > 0) {
    return externalRatings.slice(0, 4);
  }
  if (result.rating == null) {
    return [];
  }
  return [
    {
      source: result.ratingSource ?? result.ratingSources?.[0] ?? "mdblist",
      value: result.rating,
      score: result.rating,
      normalized: result.rating,
      votes: null,
      url: "",
    },
  ];
}

function runtimeLabel(runtimeMinutes: number | null) {
  if (runtimeMinutes == null || runtimeMinutes <= 0) {
    return null;
  }
  const hours = Math.floor(runtimeMinutes / 60);
  const minutes = runtimeMinutes % 60;
  if (hours <= 0) {
    return `${minutes}m`;
  }
  return minutes > 0 ? `${hours}h ${minutes}m` : `${hours}h`;
}

function SummaryRatingPill({ rating }: { rating: TitleExternalRating }) {
  const source = ratingSourceInfo(rating.source);
  const value = ratingValueLabel(rating, source);
  const content = (
    <>
      {source.logoSrc ? (
        <img
          src={source.logoSrc}
          alt=""
          aria-hidden="true"
          className="h-4 max-w-10 shrink-0 object-contain"
          loading="lazy"
        />
      ) : null}
      <span className="font-semibold text-foreground">{value}</span>
    </>
  );
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        {rating.url.trim() ? (
          <a
            href={rating.url}
            target="_blank"
            rel="noreferrer"
            aria-label={`${source.label}: ${value}`}
            className="inline-flex h-10 items-center gap-2 rounded-lg border border-border/80 bg-background/55 px-3 text-sm shadow-sm"
          >
            {content}
          </a>
        ) : (
          <span
            tabIndex={0}
            aria-label={`${source.label}: ${value}`}
            className="inline-flex h-10 items-center gap-2 rounded-lg border border-border/80 bg-background/55 px-3 text-sm shadow-sm"
          >
            {content}
          </span>
        )}
      </TooltipTrigger>
      <TooltipContent>{source.label}</TooltipContent>
    </Tooltip>
  );
}

export function CatalogActionDialogSummary({
  result,
  facet,
  mode,
}: CatalogActionDialogSummaryProps) {
  const posterUrl = selectPosterVariantUrl(result.posterUrl, "w250");
  const posterSourceUrl = selectPosterVariantUrl(result.posterUrl, "original");
  const backgroundUrl =
    selectPosterVariantUrl(result.backgroundUrl, "original") ?? posterSourceUrl;
  const ratings = ratingEntries(result);
  const links = externalLinks(result, facet);
  const genres = (result.genres ?? []).slice(0, 4);
  const runtime = runtimeLabel(result.runtimeMinutes);
  const badgeLabel = mode === "add" ? "New to Catalog" : "Request Media";
  const BadgeIcon = mode === "add" ? Sparkles : Send;

  return (
    <DialogHeader className="relative isolate overflow-hidden rounded-t-lg border-b border-border/70 p-5 text-left sm:p-7">
      {backgroundUrl ? (
        <>
          <img
            src={backgroundUrl}
            alt=""
            aria-hidden="true"
            className="absolute inset-0 -z-20 h-full w-full object-cover opacity-35 blur-xl scale-110"
          />
          <div
            aria-hidden="true"
            className="absolute inset-0 -z-10 bg-gradient-to-r from-background via-background/92 to-background/55"
          />
        </>
      ) : null}
      <div className="grid gap-5 sm:grid-cols-[minmax(120px,180px)_1fr] sm:gap-7">
        <div className="mx-auto aspect-[2/3] w-36 overflow-hidden rounded-xl border border-border/80 bg-muted shadow-2xl sm:mx-0 sm:w-full">
          {posterUrl ? (
            <TitlePoster
              src={posterUrl}
              sourceSrc={posterSourceUrl}
              alt={result.name}
              className="h-full w-full object-cover"
            />
          ) : (
            <div className="flex h-full w-full items-center justify-center px-4 text-center text-xs text-muted-foreground">
              No art
            </div>
          )}
        </div>

        <div className="min-w-0 space-y-4">
          <div className="inline-flex h-8 items-center gap-2 rounded-lg border border-primary/45 bg-primary/15 px-3 text-xs font-semibold uppercase text-primary">
            <BadgeIcon className="h-3.5 w-3.5" />
            {badgeLabel}
          </div>
          <div className="min-w-0">
            <DialogTitle className="text-3xl font-bold leading-tight tracking-normal text-foreground sm:text-4xl">
              {result.name}
              {result.year ? (
                <span className="font-semibold text-muted-foreground">
                  {" "}
                  ({result.year})
                </span>
              ) : null}
            </DialogTitle>
            <DialogDescription className="sr-only">
              {result.overview || result.name}
            </DialogDescription>
          </div>

          {ratings.length > 0 ? (
            <TooltipProvider delayDuration={200}>
              <div className="flex flex-wrap gap-2">
                {ratings.map((rating, index) => (
                  <SummaryRatingPill
                    key={`${rating.source}-${index}`}
                    rating={rating}
                  />
                ))}
              </div>
            </TooltipProvider>
          ) : null}

          {genres.length > 0 || runtime ? (
            <div className="flex flex-wrap items-center gap-2">
              {genres.map((genre) => (
                <span
                  key={genre}
                  className="rounded-lg border border-border/80 bg-background/45 px-3 py-1 text-sm font-medium text-foreground"
                >
                  {genre}
                </span>
              ))}
              {runtime ? (
                <span className="inline-flex items-center gap-1.5 px-1 py-1 text-sm font-medium text-muted-foreground">
                  <Clock3 className="h-4 w-4" />
                  {runtime}
                </span>
              ) : null}
            </div>
          ) : null}

          {result.overview ? (
            <p className="max-w-3xl text-sm leading-6 text-muted-foreground sm:text-base">
              {result.overview}
            </p>
          ) : null}

          {links.length > 0 ? (
            <div className="flex flex-wrap items-center gap-2 pt-1">
              <span className="mr-1 text-xs font-semibold uppercase text-muted-foreground">
                Open In
              </span>
              {links.map((link) => (
                <a
                  key={link.id}
                  href={link.href}
                  target="_blank"
                  rel="noreferrer"
                  className="inline-flex h-10 items-center gap-2 rounded-lg border border-border/80 bg-background/45 px-3 text-sm font-semibold text-foreground transition hover:border-primary/70 hover:bg-primary/10"
                >
                  {link.logoSrc ? (
                    <img
                      src={link.logoSrc}
                      alt=""
                      aria-hidden="true"
                      className="h-5 max-w-14 object-contain"
                      loading="lazy"
                    />
                  ) : null}
                  <span>{link.label}</span>
                  <ExternalLink className="h-3.5 w-3.5 text-muted-foreground" />
                </a>
              ))}
            </div>
          ) : null}
        </div>
      </div>
    </DialogHeader>
  );
}
