import {
  compactRatingNumber,
  ratingSourceInfo,
  ratingValueLabel,
  type RatingSourceInfo,
  type TitleExternalRating,
} from "@/lib/utils/title-ratings";

export type TitleRatings = {
  rating: number | null;
  ratingSources: string[];
  externalRatings: TitleExternalRating[];
};

type TitleRatingsStripProps = {
  ratings?: TitleRatings | null;
};

function voteLabel(votes: number | null | undefined): string | null {
  if (!votes || votes <= 0) {
    return null;
  }
  if (votes >= 1_000_000) {
    return `${compactRatingNumber(votes / 1_000_000)}M`;
  }
  if (votes >= 1_000) {
    return `${compactRatingNumber(votes / 1_000)}K`;
  }
  return votes.toString();
}

function RatingSourceLogo({ source }: { source: RatingSourceInfo }) {
  if (!source.logoSrc) {
    return null;
  }
  return (
    <img
      src={source.logoSrc}
      alt=""
      aria-hidden="true"
      className="h-3.5 w-3.5 shrink-0 object-contain"
      loading="lazy"
    />
  );
}

export function TitleRatingsStrip({ ratings }: TitleRatingsStripProps) {
  const externalRatings = ratings?.externalRatings ?? [];
  if (externalRatings.length === 0 && ratings?.rating == null) {
    return null;
  }

  return (
    <div className="mt-3 flex flex-wrap gap-2">
      {externalRatings.map((rating, index) => {
        const source = ratingSourceInfo(rating.source);
        const votes = voteLabel(rating.votes);
        const content = (
          <>
            <RatingSourceLogo source={source} />
            <span className="font-medium text-foreground">{source.label}</span>
            <span className="font-[var(--font-code)] text-card-foreground">
              {ratingValueLabel(rating, source)}
            </span>
            {votes ? <span className="text-muted-foreground/70">{votes}</span> : null}
          </>
        );
        const className =
          "inline-flex items-center gap-1.5 rounded border border-border/70 bg-background/45 px-2 py-1 text-xs";
        return rating.url.trim() ? (
          <a
            key={`${rating.source}-${index}`}
            href={rating.url}
            target="_blank"
            rel="noreferrer"
            className={className}
          >
            {content}
          </a>
        ) : (
          <span key={`${rating.source}-${index}`} className={className}>
            {content}
          </span>
        );
      })}
      {externalRatings.length === 0 && ratings?.rating != null ? (
        <span className="inline-flex items-center gap-1.5 rounded border border-border/70 bg-background/45 px-2 py-1 text-xs">
          <RatingSourceLogo source={ratingSourceInfo("mdblist")} />
          <span className="font-medium text-foreground">MDBList</span>
          <span className="font-[var(--font-code)] text-card-foreground">
            {compactRatingNumber(ratings.rating)}
          </span>
        </span>
      ) : null}
    </div>
  );
}
