import {
  TitleRatingsDisplay,
  type TitleRatingsDisplayVariant,
} from "@/components/common/title-ratings-display";
import type { TitleExternalRating } from "@/lib/utils/title-ratings";

export type TitleRatings = {
  rating: number | null;
  ratingSources: string[];
  externalRatings: TitleExternalRating[];
};

type TitleRatingsStripProps = {
  ratings?: TitleRatings | null;
  variant?: TitleRatingsDisplayVariant;
};

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
    <TitleRatingsDisplay
      externalRatings={externalRatings}
      fallbackRating={ratings?.rating}
      fallbackSource={fallbackSource}
      variant={variant}
      className={variant === "default" ? "mt-3" : "mb-3"}
    />
  );
}
