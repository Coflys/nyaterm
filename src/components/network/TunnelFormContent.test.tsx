import { describe, expect, it } from "vitest";
import type { TunnelConfig } from "@/types/global";
import { createTunnelDraft } from "./TunnelFormContent";

describe("tunnel reconnect configuration", () => {
  it("defaults new and legacy drafts to automatic reconnect", () => {
    expect(createTunnelDraft().auto_reconnect).toBe(true);
    const { auto_reconnect: _, ...legacy } = createTunnelDraft();
    expect(createTunnelDraft(legacy as TunnelConfig).auto_reconnect).toBe(true);
    expect(legacy).not.toHaveProperty("auto_reconnect");
  });

  it("preserves an explicit opt out and the user's enabled intent", () => {
    const config = {
      ...createTunnelDraft(),
      auto_reconnect: false,
      is_open: true,
    };
    expect(createTunnelDraft(config)).toEqual(config);
  });
});
