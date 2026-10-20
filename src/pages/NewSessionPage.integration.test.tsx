import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SavedConnection, SupportedSshAlgorithms } from "@/types/global";
import NewSessionPage from "./NewSessionPage";

const { invoke, close, focusListener, translate } = vi.hoisted(() => ({
  invoke: vi.fn(),
  close: vi.fn(),
  focusListener: vi.fn(async () => () => {}),
  translate: (key: string, fallback?: unknown) => (typeof fallback === "string" ? fallback : key),
}));
vi.mock("@/lib/invoke", () => ({ invoke }));
vi.mock("@/lib/backend/api", () => ({
  emit: vi.fn(async () => {}),
  listen: vi.fn(async () => () => {}),
}));
vi.mock("@/lib/backend/platform/window", () => ({
  getCurrentWindow: () => ({ close, onFocusChanged: focusListener }),
}));
vi.mock("@/context/AppContext", () => ({
  useApp: () => ({
    appSettings: {
      general: { rdp_client_mode: "builtin" },
      recording: { auto_start: false, default_mode: "transcript" },
      ui: { show_remote_stats: true },
    },
  }),
}));
vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: translate,
  }),
}));
vi.mock("@/components/panel/security-auth/PasswordManagementTab", () => ({
  PasswordManagementTab: () => <input aria-label="managed account" />,
}));
vi.mock("@/components/panel/security-auth/KeyManagementTab", () => ({
  KeyManagementTab: () => <input aria-label="managed key" />,
}));

const defaults = { kex: ["kex"], ciphers: ["cipher"], macs: ["mac"], host_keys: ["key"] };
const algorithms = {
  compatible: defaults,
  secure: defaults,
  ...Object.fromEntries(
    Object.entries(defaults).map(([key, values]) => [
      key,
      values.map((id) => ({ id, label: id, risk: "modern" })),
    ]),
  ),
} as SupportedSshAlgorithms;
let connection: SavedConnection;
const base = {
  id: "edit-id",
  name: "existing",
  host: "example.org",
  port: 22,
  username: "user",
  tags: ["saved"],
  description: "description",
};
const samples: SavedConnection[] = [
  {
    ...base,
    type: "ssh",
    auth: { mode: "password", has_password: true },
    sftp: {
      enabled: true,
      compatibility_mode: false,
      cwd_follow_mode: "shell_integration",
      shell_detection_timeout_ms: 4000,
      filename_encoding: "GBK",
      pipeline_depth: 8,
    },
  },
  {
    ...base,
    type: "telnet",
    port: 23,
    auth: { mode: "password", has_password: true },
    keep_alive_interval: 0,
    enter_mode: "lf",
    local_echo: true,
    network: { proxy_id: "proxy" },
  },
  {
    ...base,
    type: "rdp",
    port: 3389,
    domain: "DOMAIN",
    security: { use_nla: false, certificate_policy: "strict" },
    display: { mode: "fixed", width: 1280, height: 720 },
    reconnect: { enabled: false, max_attempts: 3 },
  },
  {
    ...base,
    type: "vnc",
    port: 5900,
    security: { mode: "auto" },
    display: { scale_mode: "actual" },
    view_only: true,
    shared: false,
    network: { proxy_id: "proxy" },
  },
  {
    id: "edit-id",
    name: "existing",
    type: "serial",
    port_name: "COM3",
    baud_rate: 9600,
    data_bits: 7,
    parity: "even",
    stop_bits: "2",
    flow_control: "hardware",
    modem_upload_protocol: "ymodem",
  },
  {
    id: "edit-id",
    name: "existing",
    type: "local_terminal",
    shell_path: "custom-shell",
    shell_args: "--login",
    working_dir: "C:/work",
    dynamic_tab_title: true,
  },
];

beforeEach(() => {
  history.replaceState(null, "", "/?edit=edit-id");
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
    width: 900,
    height: 600,
    x: 0,
    y: 0,
    left: 0,
    right: 900,
    top: 0,
    bottom: 600,
    toJSON: () => ({}),
  });
  connection = structuredClone(samples[0]);
  close.mockClear();
  invoke.mockReset();
  focusListener.mockClear();
  invoke.mockImplementation(async (command: string, args?: { connection?: SavedConnection }) => {
    if (command === "get_saved_connections") return [connection];
    if (command === "get_supported_ssh_algorithms") return algorithms;
    if (command === "get_default_local_shell") return "powershell.exe";
    if (command === "list_serial_ports") return ["COM3"];
    if (command === "save_connection") {
      connection = args?.connection as SavedConnection;
      return "edit-id";
    }
    if (command === "get_connection_password_value") return "secret";
    return [];
  });
});
afterEach(() => {
  vi.restoreAllMocks();
  history.replaceState(null, "", "/");
});
const field = (name: string) =>
  document.querySelector<HTMLInputElement>(`[data-editor-field="${name}"]`)!;
async function ready() {
  await waitFor(() =>
    expect(
      (screen.getByRole("button", { name: "dialog.save" }) as HTMLButtonElement).disabled,
    ).toBe(false),
  );
}
async function category(name: string) {
  await userEvent.click(
    screen.getByRole("tab", { name: new RegExp(`connectionEditor.categories.${name}`) }),
  );
}

describe("real connection editor forms", () => {
  it.each(
    samples,
  )("loads and saves every existing $type configuration through the classified editor", async (sample) => {
    connection = structuredClone(sample);
    render(<NewSessionPage />);
    await ready();
    fireEvent.change(screen.getByDisplayValue("existing"), { target: { value: "renamed" } });
    await userEvent.click(screen.getByRole("button", { name: "dialog.save" }));
    await waitFor(() => expect(close).toHaveBeenCalled());
    expect(connection.name).toBe("renamed");
    // Check protocol-specific fields rather than merely checking that save was called.
    for (const key of [
      "type",
      "host",
      "port",
      "username",
      "domain",
      "network",
      "sftp",
      "keep_alive_interval",
      "enter_mode",
      "local_echo",
      "view_only",
      "shared",
      "port_name",
      "baud_rate",
      "data_bits",
      "parity",
      "stop_bits",
      "flow_control",
      "modem_upload_protocol",
      "shell_path",
      "shell_args",
      "working_dir",
      "dynamic_tab_title",
    ] as const) {
      if (sample[key] !== undefined) expect(connection[key]).toEqual(sample[key]);
    }
  });

  it("preserves unconfirmed tags and entered fields across categories, and cancel never saves", async () => {
    render(<NewSessionPage />);
    await ready();
    fireEvent.change(screen.getByRole("combobox", { name: "dialog.tagsPlaceholder" }), {
      target: { value: "unfinished" },
    });
    fireEvent.change(field("host"), { target: { value: "changed.example.org" } });
    await category("authentication");
    expect(field("host")).toBeNull();
    const listeners = focusListener.mock.calls.length;
    await category("basic");
    expect(field("host").value).toBe("changed.example.org");
    expect(
      (screen.getByRole("combobox", { name: "dialog.tagsPlaceholder" }) as HTMLInputElement).value,
    ).toBe("unfinished");
    expect(focusListener.mock.calls.length).toBe(listeners);
    await userEvent.click(screen.getByRole("button", { name: "dialog.cancel" }));
    expect(close).toHaveBeenCalled();
    expect(invoke.mock.calls.some(([command]) => command === "save_connection")).toBe(false);
  });

  it("saves the latest blur-normalized numeric value when Enter is pressed", async () => {
    render(<NewSessionPage />);
    await ready();
    const port = field("port");
    port.focus();
    fireEvent.change(port, { target: { value: "70000" } });
    fireEvent.keyDown(port, { key: "Enter" });
    await waitFor(() => expect(connection.port).toBe(65535));
  });

  it("keeps a stored password placeholder and restores focus after account management closes", async () => {
    connection.auth = { ...connection.auth, mode: "password", account_id: "missing-account" };
    render(<NewSessionPage />);
    await ready();
    await category("authentication");
    await userEvent.click(screen.getByRole("tab", { name: "dialog.directPassword" }));
    expect(field("password").placeholder).toBe("••••••••");
    await userEvent.click(screen.getByRole("tab", { name: "dialog.accountPassword" }));
    const manage = screen.getByRole("button", { name: "dialog.manageAccounts" });
    await userEvent.click(manage);
    expect(screen.getByRole("dialog")).not.toBeNull();
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(document.activeElement).toBe(manage));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("locates hidden validation errors and resets navigation when the protocol changes or the editor reopens", async () => {
    connection.username = " ";
    const view = render(<NewSessionPage />);
    await ready();
    await userEvent.click(screen.getByRole("button", { name: "dialog.save" }));
    await waitFor(() => expect(document.activeElement).toBe(field("username")));
    expect(invoke.mock.calls.some(([command]) => command === "save_connection")).toBe(false);
    fireEvent.change(field("username"), { target: { value: "fixed" } });
    expect(screen.queryByText("Username is required")).toBeNull();
    await userEvent.click(screen.getByRole("tab", { name: "VNC" }));
    expect(field("host")).not.toBeNull();
    view.unmount();
    await act(async () => {
      render(<NewSessionPage />);
    });
    await ready();
    expect(field("host")).not.toBeNull();
  });

  it("locates the Agent value field again after reordering and removes its error after deletion", async () => {
    connection.agent_forwarding_config = {
      enabled: true,
      sources: {
        stored_keys: true,
        external_agent: true,
        external_agent_endpoints: [{ type: "auto" }, { type: "environment", variable: "" }],
      },
      policy: { mode: "allowlist", fingerprints: [] },
    };
    render(<NewSessionPage />);
    await ready();
    await userEvent.click(screen.getByRole("button", { name: "dialog.save" }));
    await waitFor(() =>
      expect(document.activeElement?.getAttribute("data-endpoint-index")).toBe("1"),
    );
    expect(document.activeElement?.tagName).toBe("INPUT");
    await userEvent.click(screen.getAllByRole("button", { name: "Move endpoint up" })[1]);
    await userEvent.click(screen.getByRole("button", { name: "dialog.save" }));
    await waitFor(() =>
      expect(document.activeElement?.getAttribute("data-endpoint-index")).toBe("0"),
    );
    await userEvent.click(screen.getAllByRole("button", { name: "Remove endpoint" })[0]);
    expect(screen.queryByText("dialog.sshAgentEndpointRequired")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "dialog.save" }));
    await waitFor(() => expect(close).toHaveBeenCalled());
  });

  it("selects the erroneous algorithm tab and preserves the secondary selection across categories", async () => {
    connection.ssh_algorithms = { mode: "custom", ...defaults, macs: ["unsupported-mac"] };
    render(<NewSessionPage />);
    await ready();
    await userEvent.click(screen.getByRole("button", { name: "dialog.save" }));
    await waitFor(() =>
      expect(
        screen.getByRole("tab", { name: "dialog.algorithmMacsTab" }).getAttribute("data-state"),
      ).toBe("active"),
    );
    expect(screen.getByRole("checkbox", { name: "unsupported-mac" })).not.toBeNull();
    await category("basic");
    await category("algorithms");
    expect(
      screen.getByRole("tab", { name: "dialog.algorithmMacsTab" }).getAttribute("data-state"),
    ).toBe("active");
    await userEvent.click(screen.getByRole("checkbox", { name: "mac" }));
    await userEvent.click(screen.getByRole("checkbox", { name: "unsupported-mac" }));
    await userEvent.click(screen.getByRole("button", { name: "dialog.save" }));
    await waitFor(() => expect(close).toHaveBeenCalled());
    expect(connection.ssh_algorithms?.macs).toEqual(["mac"]);
  });
});
