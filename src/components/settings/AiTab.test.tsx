import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { useState } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_AI_SETTINGS } from "@/lib/aiSettings";
import type { AISettings } from "@/types/global";
import { AiGeneralTab, AiModelsTab } from "./AiTab";

const { invokeMock, runtimeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  runtimeMock: { value: "desktop" as "desktop" | "web" },
}));
let currentSettings: AISettings;
let appState: Record<string, unknown>;
vi.mock("@/context/AppContext", () => ({ useApp: () => appState }));
vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));
vi.mock("@/lib/backend/runtime", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/backend/runtime")>()),
  get runtime() {
    return runtimeMock.value;
  },
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => vi.fn()) }));
vi.mock("react-i18next", async (importOriginal) => ({
  ...(await importOriginal<typeof import("react-i18next")>()),
  useTranslation: () => ({ t: (key: string) => key }),
}));

function Harness({ initial, general = false }: { initial: AISettings; general?: boolean }) {
  const [ai, setAi] = useState(initial);
  currentSettings = ai;
  appState = {
    appSettings: { ai },
    updateAppSettings: (
      patch: { ai: AISettings } | ((current: { ai: AISettings }) => { ai: AISettings }),
    ) => {
      setAi((current) => (typeof patch === "function" ? patch({ ai: current }).ai : patch.ai));
    },
  };
  return (
    <>
      {general && <AiGeneralTab />}
      <AiModelsTab />
    </>
  );
}

function proxySection() {
  return within(screen.getByText("ai.proxyTitle").closest("section") as HTMLElement);
}

function selectProxyMode(mode: "system" | "direct" | "custom") {
  fireEvent.keyDown(proxySection().getAllByRole("combobox")[0], { key: "ArrowDown" });
  fireEvent.click(
    screen.getByRole("option", { name: `ai.proxy${mode[0].toUpperCase()}${mode.slice(1)}` }),
  );
}

function settingsWithProviders(): AISettings {
  const ai = structuredClone(DEFAULT_AI_SETTINGS);
  ai.provider_credentials = ai.provider_credentials.slice(0, 2).map((credential) => ({
    ...credential,
    enabled: true,
    api_key: "test-key",
  }));
  ai.models = [
    {
      id: "openai:test-model",
      name: "test-model",
      backend: "genai",
      provider_kind: "openai",
      credential_id: null,
      enabled: true,
      source: "rust-genai",
      last_seen_at: "2026-01-01T00:00:00Z",
      supported_reasoning_efforts: ["low"],
    },
  ];
  return ai;
}

function settingsWithNoKeyProvider(kind: "ollama" | "openai_compatible"): AISettings {
  const ai = settingsWithProviders();
  ai.provider_credentials = [
    {
      ...ai.provider_credentials[0],
      id: `no-key-${kind}`,
      name: kind === "ollama" ? "Local Ollama" : "Local OpenAI-compatible",
      provider_kind: kind,
      api_protocol: kind,
      base_url: kind === "ollama" ? "http://localhost:11434/" : "http://localhost:1234/v1/",
      api_key: "",
    },
  ];
  ai.models = [];
  return ai;
}

describe("AI provider settings", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    runtimeMock.value = "desktop";
  });

  it("edits and clears built-in agent custom instructions without changing other settings", () => {
    const initial = settingsWithProviders();
    render(<Harness initial={initial} general />);
    const input = screen.getByRole("textbox", { name: "ai.agentCustomInstructions" });
    expect((input as HTMLTextAreaElement).value).toBe("");
    expect(input.getAttribute("aria-describedby")).toBe("agent-custom-instructions-desc");

    const instructions = "Prefer read-only diagnostics.\n先备份配置。";
    fireEvent.change(input, { target: { value: instructions } });
    expect(currentSettings).toEqual({ ...initial, agent_custom_instructions: instructions });
    expect((input as HTMLTextAreaElement).value).toBe(instructions);

    fireEvent.change(input, { target: { value: "" } });
    expect(currentSettings).toEqual(initial);
  });

  it("loads saved multiline agent custom instructions", () => {
    const initial = settingsWithProviders();
    initial.agent_custom_instructions = "Prefer read-only diagnostics.\n先备份配置。";
    render(<Harness initial={initial} general />);
    expect(
      (screen.getByRole("textbox", { name: "ai.agentCustomInstructions" }) as HTMLTextAreaElement)
        .value,
    ).toBe(initial.agent_custom_instructions);
  });

  it("hides agent custom instructions on Web without clearing saved instructions", () => {
    runtimeMock.value = "web";
    const initial = settingsWithProviders();
    initial.agent_custom_instructions = "Saved desktop instructions";
    render(<Harness initial={initial} general />);
    expect(screen.queryByRole("textbox", { name: "ai.agentCustomInstructions" })).toBeNull();
    expect(currentSettings).toEqual(initial);
  });

  it("edits global proxy modes and preserves custom fields while hidden", () => {
    render(<Harness initial={settingsWithProviders()} general />);
    expect(currentSettings.proxy.mode).toBe("system");
    expect(screen.queryByLabelText("settings.proxyHost")).toBeNull();
    selectProxyMode("custom");
    expect(currentSettings.proxy.no_proxy).toBe("localhost,127.0.0.1,::1");
    fireEvent.change(screen.getByLabelText("settings.proxyHost"), {
      target: { value: "proxy.example.com" },
    });
    fireEvent.change(proxySection().getByRole("spinbutton"), { target: { value: "1080" } });
    fireEvent.change(screen.getByLabelText("ai.proxyUsername"), { target: { value: "user" } });
    fireEvent.change(screen.getByLabelText("ai.proxyPassword"), { target: { value: "pass" } });
    fireEvent.change(screen.getByLabelText("ai.proxyBypass"), {
      target: { value: "localhost,internal.example.com" },
    });
    fireEvent.keyDown(proxySection().getAllByRole("combobox")[1], { key: "ArrowDown" });
    fireEvent.click(screen.getByRole("option", { name: "SOCKS5" }));
    expect(currentSettings.proxy).toMatchObject({
      protocol: "socks5",
      port: 1080,
      host: "proxy.example.com",
      username: "user",
      password: "pass",
    });
    selectProxyMode("direct");
    expect(screen.queryByLabelText("settings.proxyHost")).toBeNull();
    selectProxyMode("custom");
    expect((screen.getByLabelText("settings.proxyHost") as HTMLInputElement).value).toBe(
      "proxy.example.com",
    );
    selectProxyMode("system");
    expect(currentSettings.proxy.password).toBe("pass");
  });

  it("retains a masked proxy password while editing other fields and can explicitly clear it", () => {
    const initial = settingsWithProviders();
    initial.proxy.mode = "custom";
    initial.proxy.password = "__SET__";
    render(<Harness initial={initial} general />);
    const input = screen.getByLabelText("ai.proxyPassword") as HTMLInputElement;
    expect(input.type).toBe("password");
    expect(input.value).toBe("");
    fireEvent.change(screen.getByLabelText("settings.proxyHost"), {
      target: { value: "proxy.local" },
    });
    expect(currentSettings.proxy.password).toBe("__SET__");
    fireEvent.click(screen.getByRole("button", { name: "ai.proxyClearPassword" }));
    expect(currentSettings.proxy.password).toBe("");
    fireEvent.change(input, { target: { value: "replacement" } });
    expect(currentSettings.proxy.password).toBe("replacement");
  });

  it("ignores a pending provider result after the global proxy changes", async () => {
    let resolve!: (models: string[]) => void;
    invokeMock.mockReturnValue(
      new Promise<string[]>((done) => {
        resolve = done;
      }),
    );
    render(<Harness initial={settingsWithProviders()} general />);
    fireEvent.click(screen.getByRole("button", { name: "ai.refreshModels" }));
    selectProxyMode("direct");
    await act(async () => resolve(["late-proxy-model"]));
    expect(currentSettings.models.some((model) => model.name === "late-proxy-model")).toBe(false);
    const provider = screen.getByRole("button", { name: /^OpenAI/ });
    expect(within(provider).getByLabelText("ai.connectionStatus.idle")).toBeTruthy();
  });

  it("clears successful model tests when the proxy changes", async () => {
    invokeMock.mockResolvedValue(undefined);
    render(<Harness initial={settingsWithProviders()} general />);
    const button = screen.getByRole("button", { name: "ai.testModel" });
    await act(async () => fireEvent.click(button));
    expect(button.getAttribute("title")).toBe("ai.modelTestSucceededShort");
    selectProxyMode("direct");
    expect(button.getAttribute("title")).toBe("ai.testModel");
  });

  it("ignores a pending model test after changing the proxy", async () => {
    let resolve!: () => void;
    invokeMock.mockReturnValue(
      new Promise<void>((done) => {
        resolve = done;
      }),
    );
    render(<Harness initial={settingsWithProviders()} general />);
    const button = screen.getByRole("button", { name: "ai.testModel" }) as HTMLButtonElement;
    fireEvent.click(button);
    expect(button.disabled).toBe(true);
    selectProxyMode("direct");
    expect(button.disabled).toBe(false);
    await act(async () => resolve());
    expect(button.getAttribute("title")).toBe("ai.testModel");
  });

  it("keeps an empty account collapsed until the user adds a provider", () => {
    const initial = settingsWithProviders();
    initial.provider_credentials = [];
    initial.models = [];
    render(<Harness initial={initial} />);
    expect(screen.queryByText("ai.modelList")).toBeNull();
    expect(screen.queryByRole("button", { name: "OpenAI" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "ai.addProvider" }));
    fireEvent.click(screen.getByRole("button", { name: "OpenAI" }));
    expect(currentSettings.provider_credentials.filter((item) => item.enabled)).toHaveLength(1);
    expect(screen.getByText("ai.modelList")).toBeTruthy();
    expect(
      screen.getByRole("button", { name: "ai.addProvider" }).getAttribute("aria-expanded"),
    ).toBe("false");
  });

  it("clears stale upstream badges while retaining model configuration", async () => {
    invokeMock.mockResolvedValue([]);
    render(<Harness initial={settingsWithProviders()} />);
    await act(async () =>
      fireEvent.click(screen.getByRole("button", { name: "ai.refreshModels" })),
    );
    expect(currentSettings.models[0]).toMatchObject({
      name: "test-model",
      enabled: true,
      last_seen_at: null,
      supported_reasoning_efforts: ["low"],
    });
  });

  it("preserves default reasoning efforts when customizing an unconfigured model", () => {
    const initial = settingsWithProviders();
    delete initial.models[0].supported_reasoning_efforts;
    render(<Harness initial={initial} />);

    fireEvent.click(screen.getByRole("button", { name: "ai.editModelConfig" }));
    expect(screen.getByRole("button", { name: "none" }).getAttribute("aria-pressed")).toBe("true");
    expect(screen.getByRole("button", { name: "minimal" }).getAttribute("aria-pressed")).toBe(
      "false",
    );
    expect(screen.getByRole("button", { name: "low" }).getAttribute("aria-pressed")).toBe("true");

    fireEvent.click(screen.getByRole("button", { name: "minimal" }));
    expect(currentSettings.models[0].supported_reasoning_efforts).toEqual([
      "none",
      "minimal",
      "low",
      "medium",
      "high",
      "xhigh",
    ]);
  });

  it.each([
    ["Ollama", "ollama"],
    ["OpenAI-compatible", "openai_compatible"],
  ] as const)("allows %s without an API key to test and refresh models", async (_label, kind) => {
    invokeMock.mockResolvedValueOnce(["first-model"]).mockResolvedValueOnce(["refreshed-model"]);
    render(<Harness initial={settingsWithNoKeyProvider(kind)} />);

    expect((screen.getByPlaceholderText("settings.apiKey") as HTMLInputElement).required).toBe(
      false,
    );
    const connectionButton = screen.getByRole("button", {
      name: "ai.connectionTest",
    }) as HTMLButtonElement;
    const refreshButton = screen.getByRole("button", {
      name: "ai.refreshModels",
    }) as HTMLButtonElement;
    expect(connectionButton.disabled).toBe(false);
    expect(refreshButton.disabled).toBe(false);

    await act(async () => fireEvent.click(connectionButton));
    expect(invokeMock).toHaveBeenNthCalledWith(1, "test_ai_provider_connection", {
      aiSettings: expect.any(Object),
      credentialId: `no-key-${kind}`,
    });
    expect(currentSettings.models.some((model) => model.name === "first-model")).toBe(true);

    await act(async () => fireEvent.click(refreshButton));
    expect(invokeMock).toHaveBeenNthCalledWith(2, "test_ai_provider_connection", {
      aiSettings: expect.any(Object),
      credentialId: `no-key-${kind}`,
    });
    expect(currentSettings.models.some((model) => model.name === "refreshed-model")).toBe(true);
  });

  it("gives a second account a unique name and keeps models bound to each account", () => {
    render(<Harness initial={settingsWithProviders()} />);
    fireEvent.click(screen.getByRole("button", { name: "ai.addProvider" }));
    fireEvent.click(screen.getByRole("button", { name: "OpenAI" }));
    const providers = currentSettings.provider_credentials.filter(
      (item) => item.provider_kind === "openai",
    );
    expect(providers.map((item) => item.name)).toEqual(["OpenAI-2", "OpenAI"]);
    expect(
      currentSettings.models.find((model) => model.id === "openai:test-model")?.credential_id,
    ).toBeNull();
    expect(currentSettings.models.some((model) => model.credential_id === providers[0].id)).toBe(
      true,
    );
  });

  it("hides both detail cards after deleting the last provider", () => {
    const initial = settingsWithProviders();
    initial.provider_credentials = initial.provider_credentials.slice(0, 1);
    render(<Harness initial={initial} />);
    fireEvent.click(screen.getByRole("button", { name: "ai.deleteProvider" }));
    fireEvent.click(screen.getByRole("button", { name: "common.delete" }));
    expect(currentSettings.provider_credentials.filter((item) => item.enabled)).toHaveLength(0);
    expect(screen.queryByText("ai.modelList")).toBeNull();
    expect(screen.queryByRole("button", { name: "ai.connectionTest" })).toBeNull();
    expect(
      screen.getByRole("button", { name: "ai.addProvider" }).getAttribute("aria-expanded"),
    ).toBe("false");
  });

  it("keeps model provenance on a failed refresh", async () => {
    invokeMock.mockRejectedValue(new Error("offline"));
    render(<Harness initial={settingsWithProviders()} />);
    await act(async () =>
      fireEvent.click(screen.getByRole("button", { name: "ai.refreshModels" })),
    );
    expect(currentSettings.models[0].last_seen_at).toBe("2026-01-01T00:00:00Z");
  });

  it("resets a pending test when switching providers and ignores its late result", async () => {
    let resolve!: (models: string[]) => void;
    invokeMock.mockReturnValue(
      new Promise<string[]>((done) => {
        resolve = done;
      }),
    );
    render(<Harness initial={settingsWithProviders()} />);
    fireEvent.click(screen.getByRole("button", { name: "ai.refreshModels" }));
    const openai = screen.getByRole("button", { name: /^OpenAI/ });
    expect(within(openai).getByLabelText("ai.connectionStatus.testing")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /^Anthropic/ }));
    expect(within(openai).getByLabelText("ai.connectionStatus.idle")).toBeTruthy();
    expect(
      (screen.getByRole("button", { name: "ai.refreshModels" }) as HTMLButtonElement).disabled,
    ).toBe(false);
    await act(async () => resolve(["late-model"]));
    expect(currentSettings.models.some((model) => model.name === "late-model")).toBe(false);
    expect(within(openai).getByLabelText("ai.connectionStatus.idle")).toBeTruthy();
  });
});
