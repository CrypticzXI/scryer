export type PathMappingRow = {
  localPath: string;
  remotePath: string;
};

export type PathMappingDirection = "local-to-remote" | "remote-to-local";

export function emptyPathMappingRow(): PathMappingRow {
  return { localPath: "", remotePath: "" };
}

export function parsePathMappings(
  value: string,
  direction: PathMappingDirection,
): PathMappingRow[] {
  const rows = value
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.length > 0)
    .map((line) => {
      const [firstPath, secondPath = ""] = line.split(/=>/, 2);
      const first = firstPath.trim();
      const second = secondPath.trim();

      if (direction === "remote-to-local") {
        return { localPath: second, remotePath: first };
      }

      return { localPath: first, remotePath: second };
    });

  return rows.length > 0 ? rows : [emptyPathMappingRow()];
}

export function serializePathMappings(
  rows: PathMappingRow[],
  direction: PathMappingDirection,
): string {
  return rows
    .filter((row) => row.localPath.trim() || row.remotePath.trim())
    .map((row) =>
      direction === "remote-to-local"
        ? `${row.remotePath.trim()} => ${row.localPath.trim()}`
        : `${row.localPath.trim()} => ${row.remotePath.trim()}`,
    )
    .join("\n");
}

export function removePathMappingRow(
  rows: PathMappingRow[],
  index: number,
): PathMappingRow[] {
  const remaining = rows.filter((_, rowIndex) => rowIndex !== index);
  return remaining.length > 0 ? remaining : [emptyPathMappingRow()];
}
