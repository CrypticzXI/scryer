export type LocalPathStyle = "unix" | "windows";

function isWindowsDriveAbsolutePath(path: string): boolean {
  return /^[A-Za-z]:[\\/]/.test(path);
}

function isWindowsUncAbsolutePath(path: string): boolean {
  return /^\\\\[^\\/]+[\\/][^\\/]+/.test(path);
}

export function detectBrowserLocalPathStyle(): LocalPathStyle {
  if (typeof navigator !== "undefined") {
    const navigatorWithUserAgentData = navigator as Navigator & {
      userAgentData?: { platform?: string };
    };
    const platform =
      navigatorWithUserAgentData.userAgentData?.platform ??
      navigator.platform ??
      navigator.userAgent ??
      "";
    if (/win/i.test(platform)) {
      return "windows";
    }
  }

  return "unix";
}

export function inferLocalPathStyleFromPath(
  path: string | null | undefined,
): LocalPathStyle | null {
  const trimmed = path?.trim() ?? "";
  if (!trimmed) {
    return null;
  }

  if (isWindowsDriveAbsolutePath(trimmed) || isWindowsUncAbsolutePath(trimmed)) {
    return "windows";
  }

  if (trimmed.startsWith("/")) {
    return "unix";
  }

  return null;
}

export function resolveLocalPathStyle(
  path: string | null | undefined,
): LocalPathStyle {
  return inferLocalPathStyleFromPath(path) ?? detectBrowserLocalPathStyle();
}

export function isAbsoluteLocalPathForStyle(
  path: string,
  style: LocalPathStyle,
): boolean {
  if (style === "windows") {
    return isWindowsDriveAbsolutePath(path) || isWindowsUncAbsolutePath(path);
  }

  return path.startsWith("/");
}
