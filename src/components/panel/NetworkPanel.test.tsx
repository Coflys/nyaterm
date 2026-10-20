import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import userEvent from "@testing-library/user-event";
import { TooltipProvider } from "@/components/ui/tooltip";
import { createTunnelDraft } from "@/components/network/TunnelFormContent";
import { TunnelRow, TunnelRuntimeBadge } from "./NetworkPanel";

vi.mock("react-i18next", async (importOriginal) => ({
  ...(await importOriginal<typeof import("react-i18next")>()),
  useTranslation: () => ({
    t: (key: string, options?: { count?: number }) =>
      options?.count ? `${key} ${options.count}` : key,
  }),
}));

describe("tunnel runtime display", () => {
  it("displays the reconnect attempt count", () => {
    render(
      <TooltipProvider>
        <TunnelRuntimeBadge
          enabled
          state={{ tunnelId: "t", status: "reconnecting", retryAttempt: 7 }}
        />
      </TooltipProvider>,
    );
    expect(
      screen.getByText("network.tunnelStatusReconnectingAttempt 7"),
    ).toBeTruthy();
  });

  it.each(["starting", "reconnecting", "error"] as const)(
    "keeps an enabled %s tunnel switch available for cancellation",
    (status) => {
      const onToggle = vi.fn();
      render(
        <TooltipProvider>
          <TunnelRow
            tunnel={{ ...createTunnelDraft(), id: "t", is_open: true }}
            runtimeState={{ tunnelId: "t", status, retryAttempt: 2 }}
            groups={[]}
            onEdit={vi.fn()}
            onDelete={vi.fn()}
            onMoveGroup={vi.fn()}
            onToggle={onToggle}
          />
        </TooltipProvider>,
      );
      const toggle = screen.getByRole("switch");
      expect(toggle.getAttribute("aria-checked")).toBe("true");
      fireEvent.click(toggle);
      expect(onToggle).toHaveBeenCalledWith("t", false);
    },
  );
});

it.each(["error", "disconnected"] as const)(
  "retries an enabled %s tunnel through the existing open action",
  async (status) => {
    const onToggle = vi.fn();
    render(
      <TooltipProvider>
        <TunnelRow
          tunnel={{ ...createTunnelDraft(), id: "t", is_open: true }}
          runtimeState={{ tunnelId: "t", status, retryAttempt: 0 }}
          groups={[]}
          onEdit={vi.fn()}
          onDelete={vi.fn()}
          onMoveGroup={vi.fn()}
          onToggle={onToggle}
        />
      </TooltipProvider>,
    );
    const user = userEvent.setup();
    await user.click(screen.getByRole("button"));
    await user.click(
      screen.getByRole("menuitem", { name: "network.retryTunnel" }),
    );
    expect(onToggle).toHaveBeenCalledWith("t", true);
  },
);
