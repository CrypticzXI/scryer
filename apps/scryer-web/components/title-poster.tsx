import * as React from "react";
import type { ComponentProps } from "react";

type TitlePosterProps = Omit<ComponentProps<"img">, "src"> & {
  /** Local AVIF URL (from posterUrl). */
  src?: string | null;
  /** Source JPG URL (from posterSourceUrl) — used as <img> fallback. */
  sourceSrc?: string | null;
};

/**
 * Renders a title poster with AVIF-first, JPG-fallback via `<picture>`.
 *
 * When both `src` (local AVIF) and `sourceSrc` (original JPG) are available,
 * the browser picks AVIF if supported and falls back to the JPG otherwise.
 * When only one URL is available, it renders a plain `<img>`.
 */
export function TitlePoster({
  src,
  sourceSrc,
  alt,
  onError,
  loading = "lazy",
  decoding = "async",
  ...props
}: TitlePosterProps) {
  const avifUrl = src ?? undefined;
  const fallbackUrl = sourceSrc ?? avifUrl;
  const [preferFallbackImage, setPreferFallbackImage] = React.useState(false);

  React.useEffect(() => {
    setPreferFallbackImage(false);
  }, [avifUrl, sourceSrc]);

  if (!fallbackUrl) {
    return null;
  }

  const handleError: ComponentProps<"img">["onError"] = (event) => {
    if (avifUrl && sourceSrc) {
      setPreferFallbackImage(true);
    }
    onError?.(event);
  };

  if (avifUrl && sourceSrc && !preferFallbackImage) {
    return (
      <picture>
        <source srcSet={avifUrl} type="image/avif" />
        <img
          src={sourceSrc}
          alt={alt}
          loading={loading}
          decoding={decoding}
          onError={handleError}
          {...props}
        />
      </picture>
    );
  }

  return (
    <img
      src={fallbackUrl}
      alt={alt}
      loading={loading}
      decoding={decoding}
      onError={handleError}
      {...props}
    />
  );
}
