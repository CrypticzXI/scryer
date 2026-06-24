export function scheduleAfterFirstPaint(callback: () => void) {
  if (typeof window === "undefined") {
    callback();
    return () => {};
  }

  let cancelled = false;
  let frameId = 0;
  let idleId: number | null = null;
  let timeoutId: number | null = null;
  const idleWindow = window as Window & {
    requestIdleCallback?: (
      callback: IdleRequestCallback,
      options?: IdleRequestOptions,
    ) => number;
    cancelIdleCallback?: (handle: number) => void;
  };

  const run = () => {
    if (!cancelled) {
      callback();
    }
  };

  frameId = window.requestAnimationFrame(() => {
    if (idleWindow.requestIdleCallback) {
      idleId = idleWindow.requestIdleCallback(run, { timeout: 1_500 });
      return;
    }

    timeoutId = window.setTimeout(run, 250);
  });

  return () => {
    cancelled = true;
    if (frameId !== 0) {
      window.cancelAnimationFrame(frameId);
    }
    if (idleId != null) {
      idleWindow.cancelIdleCallback?.(idleId);
    }
    if (timeoutId != null) {
      window.clearTimeout(timeoutId);
    }
  };
}
