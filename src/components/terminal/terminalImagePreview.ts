import { getLocalPathName } from "@/components/panel/file-explorer/model";
import { invoke } from "@/lib/invoke";
import { type FileWindowTarget, openFilePreview } from "@/lib/windowManager";
import type { FileProperties, SessionType } from "@/types/global";

type PreviewBackend = "remote" | "local";

export function getTerminalImagePreviewBackend(
  sessionType: SessionType,
): PreviewBackend | null {
  if (sessionType === "SSH") return "remote";
  if (sessionType === "Local") return "local";
  return null;
}

async function expandHomePath(
  path: string,
  backend: PreviewBackend,
  sessionId: string,
) {
  if (!path.startsWith("~/")) return path;
  const home = await invoke<string>(
    backend === "local" ? "get_local_home_dir" : "get_home_dir",
    { sessionId },
  );
  return `${home.replace(/[\\/]+$/, "")}${path.slice(1)}`;
}

/** Opens an image path printed in a terminal session in the built-in preview window. */
export async function openTerminalImagePreview({
  sessionId,
  sessionType,
  path,
  target,
}: {
  sessionId: string;
  sessionType: SessionType;
  path: string;
  target?: FileWindowTarget;
}) {
  const backend = getTerminalImagePreviewBackend(sessionType);
  if (!backend) return;

  const resolvedPath = await expandHomePath(path, backend, sessionId);
  // Stat first so a stale or mistyped path reports an error instead of
  // opening an empty preview window.
  const properties = await invoke<FileProperties>(
    backend === "local" ? "get_local_file_properties" : "get_file_properties",
    { sessionId, path: resolvedPath },
  );
  if (properties.is_dir) {
    throw new Error(`${resolvedPath} is a directory`);
  }

  await openFilePreview({
    sessionId,
    backend,
    path: resolvedPath,
    name: getLocalPathName(resolvedPath, resolvedPath),
    size: properties.size,
    mtime: properties.mtime,
    target,
  });
}
