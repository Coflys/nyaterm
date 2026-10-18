import { isWindows } from "@/lib/platform";
import type { SshAgentEndpoint } from "@/types/global";

export const MAX_SSH_AGENT_FORWARDING_ENDPOINTS = 16;
export const MAX_SSH_AGENT_FORWARDING_IDENTITIES = 1024;

const MAX_SSH_AGENT_ENVIRONMENT_VARIABLE_BYTES = 255;
const MAX_SSH_AGENT_UNIX_SOCKET_PATH_BYTES = 4096;

export type SshAgentEndpointValidationError =
  | "empty"
  | "invalid"
  | "too_long"
  | "duplicate"
  | "too_many";

function utf8Length(value: string): number {
  return new TextEncoder().encode(value).length;
}

function normalizeEnvironmentVariable(value: string): string {
  return value.trim().replace(/^\$+/, "").trim();
}

function endpointKey(endpoint: SshAgentEndpoint): string {
  switch (endpoint.type) {
    case "auto":
      return isWindows ? "auto" : "environment:SSH_AUTH_SOCK";
    case "environment":
      return `environment:${normalizeEnvironmentVariable(endpoint.variable)}`;
    case "unix_socket":
      return `unix_socket:${endpoint.path}`;
    case "pageant":
      return "pageant";
    case "windows_open_ssh":
      return "windows_open_ssh";
  }
}

/** Mirrors the backend save-boundary checks so invalid endpoint drafts are visible immediately. */
export function validateSshAgentForwardingEndpoints(
  endpoints: SshAgentEndpoint[],
): SshAgentEndpointValidationError | null {
  return getSshAgentEndpointIssue(endpoints)?.code ?? null;
}

/** Frontend-only location information; the saved endpoint representation is unchanged. */
export function getSshAgentEndpointIssue(
  endpoints: SshAgentEndpoint[],
): { code: SshAgentEndpointValidationError; endpointIndex?: number } | null {
  if (endpoints.length > MAX_SSH_AGENT_FORWARDING_ENDPOINTS) {
    return { code: "too_many" };
  }

  const seen = new Set<string>();
  for (const [endpointIndex, endpoint] of endpoints.entries()) {
    if (endpoint.type === "environment") {
      const variable = normalizeEnvironmentVariable(endpoint.variable);
      if (!variable) return { code: "empty", endpointIndex };
      if (variable.includes("=") || variable.includes("\0"))
        return { code: "invalid", endpointIndex };
      if (utf8Length(variable) > MAX_SSH_AGENT_ENVIRONMENT_VARIABLE_BYTES)
        return { code: "too_long", endpointIndex };
    }
    if (endpoint.type === "unix_socket") {
      if (endpoint.path.includes("\0")) return { code: "invalid", endpointIndex };
      if (!endpoint.path.trim()) return { code: "empty", endpointIndex };
      if (utf8Length(endpoint.path) > MAX_SSH_AGENT_UNIX_SOCKET_PATH_BYTES)
        return { code: "too_long", endpointIndex };
    }

    const key = endpointKey(endpoint);
    if (seen.has(key)) return { code: "duplicate", endpointIndex };
    seen.add(key);
  }

  return null;
}
