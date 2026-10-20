import type { TFunction } from "i18next";
import { isValidSerialBaudRate, MAX_SERIAL_BAUD_RATE, MIN_SERIAL_BAUD_RATE } from "@/lib/serial";
import { getSshAgentEndpointIssue } from "@/lib/sshAgent";
import type {
  SshAgentEndpoint,
  SshAlgorithmPreferences,
  SupportedSshAlgorithms,
} from "@/types/global";
import {
  CONNECTION_CATEGORIES,
  type EditorIssue,
  type EditorProtocol,
} from "./connectionEditorModel";

export interface EditorValidationDraft {
  protocol: EditorProtocol;
  host: string;
  port: number;
  username: string;
  authType: string;
  authAgentEndpoint: SshAgentEndpoint;
  forwardingEndpoints: SshAgentEndpoint[];
  postLoginEnabled: boolean;
  postLoginCommand: string;
  postLoginDelayMs: number;
  sftpTimeout: number;
  algorithms: SshAlgorithmPreferences;
  supportedAlgorithms: SupportedSshAlgorithms | null;
  displayMode: string;
  displayWidth: number;
  displayHeight: number;
  vncSecurityMode: string;
  password: string;
  serialPortName: string;
  baudRate: string;
  shellPath: string;
}

export function validateConnectionEditor(
  draft: EditorValidationDraft,
  t: TFunction,
): EditorIssue[] {
  const issues: EditorIssue[] = [];
  const add = (category: EditorIssue["category"], field: string, message: string) =>
    issues.push({ category, field, message });
  const integerIn = (value: number, min: number, max: number) =>
    Number.isInteger(value) && value >= min && value <= max;
  if (["ssh", "telnet", "rdp", "vnc"].includes(draft.protocol)) {
    if (!draft.host.trim()) add("basic", "host", t("dialog.hostRequired"));
    if (!integerIn(draft.port, 1, 65535))
      add("basic", "port", t("dialog.portInvalid", "Port must be between 1 and 65535"));
  }
  if ((draft.protocol === "ssh" || draft.protocol === "rdp") && !draft.username.trim()) {
    add("authentication", "username", t("dialog.usernameRequired", "Username is required"));
  }
  if (draft.protocol === "ssh") {
    for (const source of ["authentication", "forwarding"] as const) {
      if (source === "authentication" && draft.authType !== "agent") continue;
      const result = getSshAgentEndpointIssue(
        source === "authentication" ? [draft.authAgentEndpoint] : draft.forwardingEndpoints,
      );
      if (!result) continue;
      const keys = {
        empty: "sshAgentEndpointRequired",
        invalid: "sshAgentEndpointInvalid",
        too_long: "sshAgentEndpointTooLong",
        duplicate: "sshAgentEndpointDuplicate",
        too_many: "sshAgentEndpointLimit",
      } as const;
      issues.push({
        category: source === "authentication" ? "authentication" : "forwarding",
        field:
          source === "authentication"
            ? "authAgentEndpoint"
            : result.endpointIndex === undefined
              ? "forwardingEndpoints"
              : "forwardingEndpoint",
        source,
        endpointIndex: source === "forwarding" ? result.endpointIndex : undefined,
        tab: source === "forwarding" ? "agent" : undefined,
        message: t(`dialog.${keys[result.code]}`),
      });
    }
    if (draft.postLoginEnabled && !draft.postLoginCommand.trim())
      add("post-login", "postLoginCommand", t("dialog.postLoginCommandRequired"));
    if (!integerIn(draft.postLoginDelayMs, 0, 60_000))
      add(
        "post-login",
        "postLoginDelayMs",
        t("dialog.postLoginDelayInvalid", { min: 0, max: 60_000 }),
      );
    if (!integerIn(draft.sftpTimeout, 100, 60_000))
      add(
        "sftp",
        "sftpTimeout",
        t("dialog.sftpShellDetectionTimeoutInvalid", { min: 100, max: 60_000 }),
      );
    if (draft.algorithms.mode === "custom") {
      for (const key of ["kex", "ciphers", "macs", "host_keys"] as const) {
        const selected = draft.algorithms[key];
        const supported =
          draft.supportedAlgorithms &&
          new Set(draft.supportedAlgorithms[key].map((algorithm) => algorithm.id));
        if (
          !selected.length ||
          (supported && selected.some((algorithm) => !supported.has(algorithm)))
        ) {
          issues.push({
            category: "algorithms",
            field: `algorithm-${key}`,
            tab: key === "host_keys" ? "host-keys" : key,
            message: t(
              selected.length
                ? "connectionEditor.unsupportedAlgorithm"
                : "connectionEditor.emptyAlgorithms",
            ),
          });
        }
      }
    }
  }
  if (draft.protocol === "rdp" && draft.displayMode === "fixed") {
    if (!integerIn(draft.displayWidth, 640, 7680))
      add("display", "displayWidth", t("dialog.rdpDisplayWidthInvalid"));
    if (!integerIn(draft.displayHeight, 480, 4320))
      add("display", "displayHeight", t("dialog.rdpDisplayHeightInvalid"));
  }
  if (
    draft.protocol === "vnc" &&
    draft.vncSecurityMode === "vnc-auth" &&
    new TextEncoder().encode(draft.password).length > 8
  ) {
    add("authentication", "password", t("dialog.vncPasswordTooLong"));
  }
  if (draft.protocol === "serial") {
    if (!draft.serialPortName.trim())
      add("serial", "serialPortName", t("dialog.serialPortRequired", "Serial port is required"));
    if (!isValidSerialBaudRate(draft.baudRate))
      add(
        "serial",
        "baudRate",
        t("dialog.baudRateInvalid", {
          min: MIN_SERIAL_BAUD_RATE,
          max: MAX_SERIAL_BAUD_RATE,
        }),
      );
  }
  if (draft.protocol === "local" && !draft.shellPath.trim())
    add("startup", "shellPath", t("dialog.shellPathRequired", "Shell path is required"));
  const order: readonly string[] = CONNECTION_CATEGORIES[draft.protocol];
  return issues.sort((left, right) => order.indexOf(left.category) - order.indexOf(right.category));
}
