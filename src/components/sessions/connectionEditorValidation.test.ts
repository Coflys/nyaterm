import { createInstance } from "i18next";
import { describe, expect, it } from "vitest";
import en from "@/i18n/locales/en.json";
import { CONNECTION_CATEGORIES, type EditorProtocol } from "./connectionEditorModel";
import { type EditorValidationDraft, validateConnectionEditor } from "./connectionEditorValidation";

const i18n = createInstance();
await i18n.init({ lng: "en", resources: { en: { translation: en } } });
const defaults = { kex: ["kex"], ciphers: ["cipher"], macs: ["mac"], host_keys: ["key"] };
const draft: EditorValidationDraft = {
  protocol: "ssh",
  host: "example.org",
  port: 22,
  username: "root",
  authType: "password",
  authAgentEndpoint: { type: "auto" },
  forwardingEndpoints: [],
  postLoginEnabled: false,
  postLoginCommand: "",
  postLoginDelayMs: 1000,
  sftpTimeout: 3000,
  algorithms: { mode: "compatible", ...defaults },
  supportedAlgorithms: {
    ...Object.fromEntries(
      Object.entries(defaults).map(([key, values]) => [
        key,
        values.map((id) => ({ id, label: id, risk: "modern" })),
      ]),
    ),
    compatible: defaults,
    secure: defaults,
  } as EditorValidationDraft["supportedAlgorithms"],
  displayMode: "fit-window",
  displayWidth: 1920,
  displayHeight: 1080,
  vncSecurityMode: "auto",
  password: "",
  serialPortName: "COM3",
  baudRate: "115200",
  shellPath: "powershell.exe",
};
const validate = (changes: Partial<EditorValidationDraft>) =>
  validateConnectionEditor({ ...draft, ...changes }, i18n.t);

describe("connection editor validation", () => {
  it.each(
    Object.keys(CONNECTION_CATEGORIES) as EditorProtocol[],
  )("accepts an existing valid %s draft", (protocol) => {
    expect(validate({ protocol })).toEqual([]);
  });

  it("orders errors by navigation, independently of validation implementation order", () => {
    expect(
      validate({
        host: "",
        username: "",
        sftpTimeout: 0,
        postLoginEnabled: true,
        forwardingEndpoints: [{ type: "environment", variable: "" }],
      }).map(({ category, field }) => [category, field]),
    ).toEqual([
      ["basic", "host"],
      ["authentication", "username"],
      ["sftp", "sftpTimeout"],
      ["forwarding", "forwardingEndpoint"],
      ["post-login", "postLoginCommand"],
    ]);
  });

  it("retains endpoint source and recalculates the index after reorder and removal", () => {
    const good = { type: "environment", variable: "SSH_AUTH_SOCK" } as const;
    const bad = { type: "environment", variable: "" } as const;
    expect(validate({ forwardingEndpoints: [good, bad] })[0]).toMatchObject({
      source: "forwarding",
      endpointIndex: 1,
      tab: "agent",
    });
    expect(validate({ forwardingEndpoints: [bad, good] })[0]).toMatchObject({ endpointIndex: 0 });
    expect(validate({ forwardingEndpoints: [good] })).toEqual([]);
    expect(validate({ authType: "agent", authAgentEndpoint: bad })[0]).toMatchObject({
      source: "authentication",
      field: "authAgentEndpoint",
    });
    expect(validate({ forwardingEndpoints: [good, good] })[0]).toMatchObject({ endpointIndex: 1 });
  });

  it("locates empty and unsupported algorithm lists in their own secondary tabs", () => {
    expect(
      validate({
        algorithms: { mode: "custom", ...defaults, ciphers: [], host_keys: ["unknown"] },
      }).map(({ field, tab }) => [field, tab]),
    ).toEqual([
      ["algorithm-ciphers", "ciphers"],
      ["algorithm-host_keys", "host-keys"],
    ]);
  });

  it("keeps classic VNC's byte limit separate from other security modes", () => {
    expect(
      validate({ protocol: "vnc", vncSecurityMode: "vnc-auth", password: "中文密码" })[0],
    ).toMatchObject({ category: "authentication", field: "password" });
    expect(validate({ protocol: "vnc", password: "中文密码" })).toEqual([]);
  });
});
