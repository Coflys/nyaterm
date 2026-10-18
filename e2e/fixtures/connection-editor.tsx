import { createInstance } from "i18next";
import { createRoot } from "react-dom/client";
import { I18nextProvider } from "react-i18next";
import { ChildAppProvider } from "../../src/context/ChildAppProvider";
import NewSessionPage from "../../src/pages/NewSessionPage";
import en from "../../src/i18n/locales/en.json";
import ru from "../../src/i18n/locales/ru.json";
import zhCN from "../../src/i18n/locales/zh-CN.json";
import "../../src/index.css";

const params = new URLSearchParams(location.search);
const defaults = {
  kex: ["kex-0"],
  ciphers: ["cipher-0"],
  macs: ["mac-0"],
  host_keys: ["key-0"],
};
const algorithms = {
  compatible: defaults,
  secure: defaults,
  ...Object.fromEntries(
    Object.entries(defaults).map(([key, [first]]) => [
      key,
      Array.from({ length: 40 }, (_, index) => ({
        id: first.replace("0", String(index)),
        label: `Algorithm ${index} ${"long-name-".repeat(6)}`,
        risk: "modern",
      })),
    ]),
  ),
};
const common = {
  id: "fixture",
  name: "Fixture connection",
  host: "example.org",
  username: "user",
  tags: ["production"],
};
const protocol = params.get("protocol") ?? "ssh";
const connection = {
  ...common,
  type: protocol === "local" ? "local_terminal" : protocol,
  port: { ssh: 22, telnet: 23, rdp: 3389, vnc: 5900 }[protocol],
  port_name: "COM3",
  baud_rate: 115200,
  shell_path: "powershell.exe",
  auth: { mode: "password", has_password: true },
  ssh_algorithms: { mode: "custom", ...defaults },
  agent_forwarding_config: {
    enabled: true,
    sources: {
      stored_keys: true,
      external_agent: true,
      external_agent_endpoints: [
        { type: "windows_open_ssh" },
        { type: "pageant" },
        { type: "auto" },
      ],
    },
    policy: { mode: "allowlist", fingerprints: [] },
  },
};
const fixtureWindow = window as unknown as {
  editorFixtureInvoke: (
    command: string,
    args?: Record<string, unknown>,
  ) => Promise<unknown>;
  editorSaves: unknown[];
  editorClosed: boolean;
};
fixtureWindow.editorSaves = [];
fixtureWindow.editorFixtureInvoke = async (command, args) => {
  if (command === "get_app_settings") throw new Error("Use fixture defaults");
  if (command === "get_app_lock_state") return false;
  if (command === "get_saved_connections") return [connection];
  if (command === "get_supported_ssh_algorithms") return algorithms;
  if (command === "get_default_local_shell") return "powershell.exe";
  if (command === "list_serial_ports") return ["COM3"];
  if (command === "get_connection_password_value") return "fixture-password";
  if (command === "save_connection") {
    fixtureWindow.editorSaves.push(args?.connection);
    return "fixture";
  }
  if (command === "plugin:window|close") {
    fixtureWindow.editorClosed = true;
    return null;
  }
  if (command === "plugin:event|listen") return 1;
  if (command.startsWith("plugin:")) return null;
  if (command === "get_ssh_agent_forwarding_identities")
    return { identities: [], endpoint_errors: [] };
  return [];
};
const originalFetch = window.fetch.bind(window);
window.fetch = async (input, init) => {
  const url = new URL(
    typeof input === "string" || input instanceof URL ? input : input.url,
    location.href,
  );
  if (url.pathname.startsWith("/api/commands/")) {
    try {
      const result = await fixtureWindow.editorFixtureInvoke(
        url.pathname.split("/").at(-1)!,
        init?.body ? JSON.parse(String(init.body)) : undefined,
      );
      return Response.json(result);
    } catch (error) {
      return Response.json({ error: String(error) }, { status: 500 });
    }
  }
  return originalFetch(input, init);
};

if (params.has("host")) {
  const { WebviewWindow } =
    await import("../../src/lib/backend/platform/webviewWindow");
  new WebviewWindow("new-session", {
    url: `/e2e/fixtures/connection-editor.html?web=1&edit=fixture&lang=${params.get("lang") ?? "en"}`,
    width: 900,
    height: 680,
  });
} else {
  const style = document.createElement("style");
  style.textContent = `html { font-size: ${Number(params.get("fontSize") ?? 16)}px !important; }`;
  document.head.append(style);
  const i18n = createInstance();
  await i18n.init({
    lng: params.get("lang") ?? "en",
    fallbackLng: "en",
    resources: {
      en: { translation: en },
      ru: { translation: ru },
      "zh-CN": { translation: zhCN },
    },
  });
  createRoot(document.getElementById("root")!).render(
    <ChildAppProvider>
      <I18nextProvider i18n={i18n}>
        <NewSessionPage />
      </I18nextProvider>
    </ChildAppProvider>,
  );
}
