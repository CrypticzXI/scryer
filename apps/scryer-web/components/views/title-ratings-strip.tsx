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
import { cn } from "@/lib/utils";

export type TitleRatings = {
  rating: number | null;
  ratingSources: string[];
  externalRatings: TitleExternalRating[];
};

type TitleRatingsStripProps = {
  ratings?: TitleRatings | null;
  variant?: "default" | "hero";
};

function RatingSourceLogo({
  source,
  variant,
}: {
  source: RatingSourceInfo;
  variant: "default" | "hero";
}) {
  if (!source.logoSrc) {
    return null;
  }
  return (
    <img
      src={source.logoSrc}
      alt=""
      aria-hidden="true"
      className={cn(
        "shrink-0 object-contain",
        variant === "hero" ? "h-4 w-4" : "h-3.5 w-3.5",
      )}
      loading="lazy"
    />
  );
}

function RatingPill({
  source,
  value,
  href,
  variant,
}: {
  source: RatingSourceInfo;
  value: string;
  href: string;
  variant: "default" | "hero";
}) {
  const className = cn(
    "inline-flex items-center gap-1.5 rounded border",
    variant === "hero"
      ? "border-white/15 bg-black/35 px-2.5 py-1.5 text-sm shadow-[inset_0_1px_0_rgba(255,255,255,0.08)] backdrop-blur-sm"
      : "border-border/70 bg-background/45 px-2 py-1 text-xs",
  );
  const label = `${source.label}: ${value}`;
  const content = (
    <>
      <RatingSourceLogo source={source} variant={variant} />
      <span
        className={cn(
          "font-[var(--font-code)]",
          variant === "hero" ? "text-white" : "text-card-foreground",
        )}
      >
        {value}
      </span>
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

export function TitleRatingsStrip({
  ratings,
  variant = "default",
}: TitleRatingsStripProps) {
  const externalRatings = ratings?.externalRatings ?? [];
  if (externalRatings.length === 0 && ratings?.rating == null) {
    return null;
  }
  const fallbackSource = ratings?.ratingSources.find(
    (source) => source.trim().length > 0,
  );

  return (
    <TooltipProvider delayDuration={200}>
      <div
        className={cn(
          "flex flex-wrap gap-2",
          variant === "default" ? "mt-3" : "mb-3",
        )}
      >
        {externalRatings.map((rating, index) => {
          const source = ratingSourceInfo(rating.source);
          return (
            <RatingPill
              key={`${rating.source}-${index}`}
              source={source}
              value={ratingValueLabel(rating, source)}
              href={rating.url}
              variant={variant}
            />
          );
        })}
        {externalRatings.length === 0 && ratings?.rating != null ? (
          <RatingPill
            source={ratingSourceInfo(fallbackSource ?? "mdblist")}
            value={compactRatingNumber(ratings.rating)}
            href=""
            variant={variant}
          />
        ) : null}
      </div>
    </TooltipProvider>
  );
}
