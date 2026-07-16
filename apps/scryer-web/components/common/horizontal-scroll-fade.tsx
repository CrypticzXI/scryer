import * as React from "react";

import { cn } from "@/lib/utils";

type HorizontalScrollFadeProps = {
  children: React.ReactNode;
  className?: string;
  containerClassName?: string;
  fadeClassName?: string;
};

export function HorizontalScrollFade({
  children,
  className,
  containerClassName,
  fadeClassName,
}: HorizontalScrollFadeProps) {
  const scrollRef = React.useRef<HTMLDivElement>(null);
  const [hasMoreRight, setHasMoreRight] = React.useState(false);

  const updateOverflow = React.useCallback(() => {
    const element = scrollRef.current;
    if (!element) {
      return;
    }

    setHasMoreRight(element.scrollWidth - element.clientWidth - element.scrollLeft > 1);
  }, []);

  React.useEffect(() => {
    const element = scrollRef.current;
    if (!element) {
      return;
    }

    updateOverflow();
    if (typeof ResizeObserver === "undefined") {
      return;
    }

    const resizeObserver = new ResizeObserver(updateOverflow);
    resizeObserver.observe(element);
    return () => resizeObserver.disconnect();
  }, [children, updateOverflow]);

  return (
    <div className={cn("relative min-w-0", containerClassName)}>
      <div ref={scrollRef} className={className} onScroll={updateOverflow}>
        {children}
      </div>
      {hasMoreRight ? (
        <div
          aria-hidden="true"
          className={cn(
            "pointer-events-none absolute inset-y-0 right-0 z-10 w-14 bg-gradient-to-r from-transparent to-[var(--scry-surf)]",
            fadeClassName,
          )}
        />
      ) : null}
    </div>
  );
}
