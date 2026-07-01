import type { SeriesMovieLink } from "@/components/containers/series-overview-container";
import { useTranslate } from "@/lib/context/translate-context";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";
import { TitlePoster } from "@/components/title-poster";
import { localizedTitleStatus } from "../overview-localization";
import {
  AnidbExternalLink,
  ImdbExternalLink,
  MalExternalLink,
  TmdbExternalLink,
  TvdbMovieExternalLink,
} from "@/components/common/external-media-links";
import { formatRuntimeFromMinutes } from "./helpers";

type SeriesMoviePanelProps = {
  link: SeriesMovieLink;
  hasFile?: boolean;
};

export function SeriesMoviePanel({ link, hasFile }: SeriesMoviePanelProps) {
  const t = useTranslate();
  const movie = link.movie;
  const runtime = formatRuntimeFromMinutes(movie.runtimeMinutes);
  const posterUrl = selectPosterVariantUrl(movie.posterUrl, "w250");
  const badges = buildMovieBadges(link, hasFile, t);
  const localizedStatus = localizedTitleStatus(t, movie.contentStatus);

  return (
    <div className="flex flex-col gap-4 sm:flex-row sm:items-start">
      <div className="shrink-0">
        {posterUrl ? (
          <TitlePoster
            src={posterUrl}
            alt={movie.title}
            className="h-auto w-28 rounded-lg object-cover shadow-md sm:w-[140px]"
          />
        ) : (
          <div className="flex h-40 w-28 items-center justify-center rounded-lg bg-muted text-sm text-muted-foreground/60 sm:h-[210px] sm:w-[140px]">
            {t("title.noPoster")}
          </div>
        )}
      </div>
      <div className="min-w-0 flex-1">
        <p className="text-sm font-semibold text-card-foreground">{movie.title}</p>
        {badges.length > 0 ? (
          <div className="mt-2 flex flex-wrap gap-2">
            {badges.map((badge) => (
              <span
                key={`${badge.label}-${badge.tone}`}
                className={`rounded-full border px-2 py-0.5 text-[11px] font-medium ${badgeClassName(badge.tone)}`}
              >
                {badge.label}
              </span>
            ))}
          </div>
        ) : null}
        <div className="mt-1 flex flex-wrap gap-2 text-xs text-muted-foreground">
          {movie.year ? <span>{movie.year}</span> : null}
          {runtime ? <span>{runtime}</span> : null}
          {localizedStatus ? <span>{localizedStatus}</span> : null}
        </div>
        {movie.overview ? (
          <p className="mt-3 text-sm leading-relaxed text-muted-foreground">{movie.overview}</p>
        ) : (
          <p className="mt-3 text-sm italic text-muted-foreground/60">{t("title.descriptionUnavailable")}</p>
        )}
        {link.signalSummary ? (
          <p className="mt-2 text-xs text-muted-foreground/80">{link.signalSummary}</p>
        ) : null}
        <div className="mt-3 flex flex-wrap gap-3 text-sm">
          <ImdbExternalLink imdbId={movie.imdbId} />
          <TvdbMovieExternalLink tvdbId={movie.tvdbId} slug={movie.slug} />
          <TmdbExternalLink mediaType="movie" tmdbId={movie.tmdbId} />
          <MalExternalLink malId={movie.malId} />
          <AnidbExternalLink anidbId={movie.anidbId} />
        </div>
      </div>
    </div>
  );
}

function buildMovieBadges(
  link: SeriesMovieLink,
  hasFile: boolean | undefined,
  t: (key: string, values?: Record<string, string | number | boolean | null | undefined>) => string,
): Array<{ label: string; tone: "emerald" | "amber" | "slate" | "red" }> {
  const badges: Array<{ label: string; tone: "emerald" | "amber" | "slate" | "red" }> = [];

  if (hasFile === true) {
    badges.push({ label: t("history.downloadCompleted"), tone: "emerald" });
  } else if (link.monitored) {
    badges.push({ label: t("episode.missing"), tone: "red" });
  } else {
    badges.push({ label: t("search.monitorType.unmonitored"), tone: "slate" });
  }

  if (link.movieForm === "recap") {
    badges.push({ label: t("episode.recap"), tone: "slate" });
  } else if (link.movieForm === "special") {
    badges.push({ label: t("episode.special"), tone: "slate" });
  } else if (link.continuityStatus === "filler") {
    badges.push({ label: t("episode.filler"), tone: "slate" });
  } else if (link.continuityStatus === "canon") {
    badges.push({ label: t("title.canon"), tone: "emerald" });
  } else if (link.continuityStatus === "mixed") {
    badges.push({ label: t("title.mixed"), tone: "amber" });
  }

  return badges;
}

function badgeClassName(tone: "emerald" | "amber" | "slate" | "red") {
  switch (tone) {
    case "emerald":
      return "border-emerald-500/30 bg-emerald-500/10 text-emerald-200";
    case "amber":
      return "border-amber-500/30 bg-amber-500/10 text-amber-100";
    case "red":
      return "border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] text-[var(--scry-danger-text)]";
    default:
      return "border-border bg-muted/30 text-muted-foreground";
  }
}
