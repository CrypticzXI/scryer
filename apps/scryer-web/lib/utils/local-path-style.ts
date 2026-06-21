export type LocalPathStyle = "unix" | "windows";
export type RuntimePathStyleValue = "UNIX" | "WINDOWS";

function isWindowsDriveAbsolutePath(path: string): boolean {
  return /^[A-Za-z]:[\\/]/.test(path);
}

function isWindowsUncAbsolutePath(path: string): boolean {
  return /^\\\\[^\\/]+[\\/][^\\/]+/.test(path);
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

export function localPathStyleFromRuntimeValue(
  value: string | null | undefined,
): LocalPathStyle {
  return value === "WINDOWS" ? "windows" : "unix";
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
