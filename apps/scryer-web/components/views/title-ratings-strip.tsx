import {
  TitleRatingsDisplay,
  type TitleRatingsDisplayVariant,
} from "@/components/common/title-ratings-display";
import {
  normalizeTitleExternalRating,
  type TitleExternalRatingInput,
} from "@/lib/utils/title-ratings";

export type TitleRatings = {
  rating?: number | null;
  ratingSources?: string[];
  externalRatings?: TitleExternalRatingInput[];
};

type TitleRatingsStripProps = {
  ratings?: TitleRatings | null;
  variant?: TitleRatingsDisplayVariant;
};

export function TitleRatingsStrip({
  ratings,
  variant = "default",
}: TitleRatingsStripProps) {
  const externalRatings = (ratings?.externalRatings ?? []).map(
    normalizeTitleExternalRating,
  );
  const fallbackRating = ratings?.rating ?? null;
  if (externalRatings.length === 0 && fallbackRating == null) {
    return null;
  }
  const fallbackSource = ratings?.ratingSources?.find(
    (source) => source.trim().length > 0,
  );

  return (
    <TitleRatingsDisplay
      externalRatings={externalRatings}
      fallbackRating={fallbackRating}
      fallbackSource={fallbackSource}
      variant={variant}
      className={variant === "default" ? "mt-3" : "mb-3"}
    />
  );
}
