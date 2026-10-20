import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { SerialFlowControl } from "@/types/global";
import { SerialForm } from "./SerialForm";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

function props(flowControl: SerialFlowControl) {
  return {
    serialPortName: "COM3",
    setSerialPortName: vi.fn(),
    serialPortOptions: [{ value: "COM3" }],
    serialPortsLoading: false,
    serialPortsError: "",
    onSerialPortDropdownOpen: vi.fn(),
    baudRate: "115200",
    setBaudRate: vi.fn(),
    dataBits: "8",
    setDataBits: vi.fn(),
    parity: "none",
    setParity: vi.fn(),
    stopBits: "1",
    setStopBits: vi.fn(),
    flowControl,
    setFlowControl: vi.fn(),
    backspaceMode: "ctrl_h",
    setBackspaceMode: vi.fn(),
    modemUploadProtocol: "zmodem" as const,
    setModemUploadProtocol: vi.fn(),
    recordingUseGlobal: true,
    setRecordingUseGlobal: vi.fn(),
    recordingAutoStart: false,
    setRecordingAutoStart: vi.fn(),
    recordingMode: "transcript" as const,
    setRecordingMode: vi.fn(),
    encoding: "global",
    setEncoding: vi.fn(),
  };
}

describe("SerialForm flow control", () => {
  it.each([
    "none",
    "software",
    "hardware",
  ] as const)("shows the conflict explanation and disables modem selection only for software (%s)", (flowControl) => {
    render(<SerialForm {...props(flowControl)} />);
    expect(screen.getByRole("combobox", { name: "dialog.serialFlowControl" })).not.toBeNull();
    expect(screen.queryByText("dialog.serialSoftwareFlowControlModemDisabled") !== null).toBe(
      flowControl === "software",
    );
    const protocolSelect = screen.getByText("ZMODEM").closest("button");
    expect(protocolSelect?.disabled).toBe(flowControl === "software");
  });
});
