import type { ComponentProps } from "react";

type TitlePosterProps = Omit<ComponentProps<"img">, "src"> & {
  /** Scryer-owned image URL. The server resolves local, cached, or fallback bytes. */
  src?: string | null;
};

/** Renders the single Scryer image endpoint selected by the caller. */
export function TitlePoster({
  src,
  alt,
  onError,
  loading = "lazy",
  decoding = "async",
  ...props
}: TitlePosterProps) {
  if (!src) {
    return null;
  }

  return (
    <img
      src={src}
      alt={alt}
      loading={loading}
      decoding={decoding}
      onError={onError}
      {...props}
    />
  );
}
