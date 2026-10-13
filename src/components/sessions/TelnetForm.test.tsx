import { fireEvent, render, screen } from "@testing-library/react";
import { useState, type ComponentProps } from "react";
import { describe, expect, it, vi } from "vitest";
import { TelnetForm } from "./TelnetForm";

vi.mock("react-i18next", async (importOriginal) => ({
  ...(await importOriginal<typeof import("react-i18next")>()),
  useTranslation: () => ({ t: (key: string) => key }),
}));
vi.mock("@/lib/invoke", () => ({ invoke: vi.fn().mockResolvedValue([]) }));
vi.mock("@/lib/backend/platform/window", () => ({
  getCurrentWindow: () => ({ onFocusChanged: () => Promise.resolve(() => {}) }),
}));

function Harness() {
  const [rawTcpCli, setRawTcpCli] = useState(false);
  const [keepAliveInterval, setKeepAliveInterval] = useState(60);
  const props: ComponentProps<typeof TelnetForm> = {
    host: "example.com",
    setHost: vi.fn(),
    port: 23,
    setPort: vi.fn(),
    username: "",
    setUsername: vi.fn(),
    accountId: "",
    setAccountId: vi.fn(),
    accounts: [],
    onAccountsChanged: vi.fn(),
    passwordSource: "direct",
    setPasswordSource: vi.fn(),
    authType: "none",
    setAuthType: vi.fn(),
    password: "",
    setPassword: vi.fn(),
    hasPassword: false,
    setHasPassword: vi.fn(),
    backspaceMode: "del",
    setBackspaceMode: vi.fn(),
    rawTcpCli,
    setRawTcpCli,
    keepAliveInterval,
    setKeepAliveInterval,
    enterMode: "cr",
    setEnterMode: vi.fn(),
    localEcho: false,
    setLocalEcho: vi.fn(),
    localLineEdit: false,
    setLocalLineEdit: vi.fn(),
    forceCharacterAtATime: false,
    setForceCharacterAtATime: vi.fn(),
    sendNaws: true,
    setSendNaws: vi.fn(),
    sendSga: true,
    setSendSga: vi.fn(),
    recordingUseGlobal: true,
    setRecordingUseGlobal: vi.fn(),
    recordingAutoStart: false,
    setRecordingAutoStart: vi.fn(),
    recordingMode: "transcript",
    setRecordingMode: vi.fn(),
    encoding: "global",
    setEncoding: vi.fn(),
  };
  return (
    <>
      <button onClick={() => setRawTcpCli(!rawTcpCli)}>toggle-raw</button>
      <TelnetForm {...props} />
    </>
  );
}

function openCompatibility() {
  fireEvent.click(screen.getByText("dialog.advancedConfig"));
  fireEvent.mouseDown(
    screen.getByRole("tab", { name: "dialog.telnetCompatibility" }),
    { button: 0, ctrlKey: false },
  );
  return screen.getByRole("spinbutton", {
    name: "dialog.telnetKeepAliveInterval",
  }) as HTMLInputElement;
}

describe("Telnet NOP keep-alive settings", () => {
  it("allows zero, clamps negative values, and retains the value across Raw TCP mode", () => {
    render(<Harness />);
    const input = openCompatibility();
    expect(input.value).toBe("60");
    fireEvent.change(input, { target: { value: "0" } });
    expect(input.value).toBe("0");
    fireEvent.change(input, { target: { value: "-1" } });
    fireEvent.blur(input);
    expect(input.value).toBe("0");
    fireEvent.change(input, { target: { value: "17" } });
    expect(input.value).toBe("17");
    fireEvent.click(screen.getByText("toggle-raw"));
    expect(input.disabled).toBe(true);
    expect(input.value).toBe("17");
    fireEvent.click(screen.getByText("toggle-raw"));
    expect(input.disabled).toBe(false);
    expect(input.value).toBe("17");
  });
});
