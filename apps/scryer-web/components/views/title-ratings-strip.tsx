import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
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

function RatingPill({
  source,
  value,
  href,
}: {
  source: RatingSourceInfo;
  value: string;
  href: string;
}) {
  const className =
    "inline-flex items-center gap-1.5 rounded border border-border/70 bg-background/45 px-2 py-1 text-xs";
  const label = `${source.label}: ${value}`;
  const content = (
    <>
      <RatingSourceLogo source={source} />
      <span className="font-[var(--font-code)] text-card-foreground">{value}</span>
    </>
  );

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        {href.trim() ? (
          <a
            href={href}
            target="_blank"
            rel="noreferrer"
            aria-label={label}
            className={className}
          >
            {content}
          </a>
        ) : (
          <span tabIndex={0} aria-label={label} className={className}>
            {content}
          </span>
        )}
      </TooltipTrigger>
      <TooltipContent>{source.label}</TooltipContent>
    </Tooltip>
  );
}

export function TitleRatingsStrip({ ratings }: TitleRatingsStripProps) {
  const externalRatings = ratings?.externalRatings ?? [];
  if (externalRatings.length === 0 && ratings?.rating == null) {
    return null;
  }

  return (
    <TooltipProvider delayDuration={200}>
      <div className="mt-3 flex flex-wrap gap-2">
        {externalRatings.map((rating, index) => {
          const source = ratingSourceInfo(rating.source);
          return (
            <RatingPill
              key={`${rating.source}-${index}`}
              source={source}
              value={ratingValueLabel(rating, source)}
              href={rating.url}
            />
          );
        })}
        {externalRatings.length === 0 && ratings?.rating != null ? (
          <RatingPill
            source={ratingSourceInfo("mdblist")}
            value={compactRatingNumber(ratings.rating)}
            href=""
          />
        ) : null}
      </div>
    </TooltipProvider>
  );
}
