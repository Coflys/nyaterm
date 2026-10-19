export const CONNECTION_CATEGORIES = {
  ssh: [
    "basic",
    "authentication",
    "network",
    "terminal",
    "sftp",
    "forwarding",
    "post-login",
    "algorithms",
  ],
  telnet: ["basic", "authentication", "network", "terminal", "compatibility"],
  rdp: ["basic", "authentication", "network", "security", "display", "session"],
  vnc: ["basic", "authentication", "network", "security", "display", "session"],
  serial: ["basic", "serial", "transfer"],
  local: ["basic", "startup", "terminal"],
} as const;

export type EditorProtocol = keyof typeof CONNECTION_CATEGORIES;
export type EditorCategory = (typeof CONNECTION_CATEGORIES)[EditorProtocol][number];
export type AlgorithmTab = "kex" | "ciphers" | "macs" | "host-keys";
export type ForwardingTab = "agent" | "x11";

export interface EditorIssue {
  category: EditorCategory;
  field: string;
  message: string;
  tab?: AlgorithmTab | ForwardingTab;
  endpointIndex?: number;
  source?: "authentication" | "forwarding";
}

export function editorCategoryKey(protocol: EditorProtocol, category: EditorCategory) {
  return `connectionEditor.categories.${protocol === "rdp" && category === "session" ? "rdpSession" : category}`;
}
