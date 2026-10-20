import { expect, test } from "@playwright/test";

// Exercise the production React surface and browser WebSocket adapter with a
// controlled server stream. This does not replace the deployed-server/Windows soak.
function frame(sequence, full = true, color = [255, 0, 0, 255]) {
  const width = full ? 2 : 1;
  const buffer = Buffer.alloc(44 + width * 4);
  buffer.writeBigUInt64LE(BigInt(sequence));
  [2, 1, full ? 0 : 1, 0, width, 1, width * 4, 2, width * 4].forEach(
    (value, index) => buffer.writeUInt32LE(value, 8 + index * 4),
  );
  for (let x = 0; x < width; x++) buffer.set(color, 44 + x * 4);
  return buffer;
}
async function streams(page) {
  const sockets = new Map();
  const counts = new Map();
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.routeWebSocket("**/api/sessions/*/vnc", (socket) => {
    const id = new URL(socket.url()).pathname.split("/").at(-2);
    counts.set(id, (counts.get(id) ?? 0) + 1);
    sockets.set(id, socket);
    socket.send(frame(1));
    socket.send(
      JSON.stringify({
        type: "event",
        event: `vnc-state-${id}`,
        payload: { sessionId: id, state: "active" },
      }),
    );
  });
  return { sockets, counts, errors };
}
async function pixels(page, id = "primary") {
  return page
    .getByTestId(id)
    .locator("canvas")
    .evaluate((canvas) =>
      Array.from(canvas.getContext("2d").getImageData(0, 0, 2, 1).data),
    );
}
const red = [255, 0, 0, 255, 255, 0, 0, 255];

test("hidden Web panes retain sockets beyond 30 seconds and restore exact pixels after moves", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const getContext = HTMLCanvasElement.prototype.getContext;
    HTMLCanvasElement.prototype.getContext = function (type, ...args) {
      return type === "webgl2" ? null : getContext.call(this, type, ...args);
    };
  });
  const { sockets, counts, errors } = await streams(page);
  await page.goto("/e2e/fixtures/vnc.html");
  await expect.poll(() => pixels(page)).toEqual(red);
  await expect.poll(() => pixels(page, "secondary")).toEqual(red);
  const attached = counts.get("primary");
  const original = sockets.get("primary");
  await page.getByRole("button", { name: "Toggle visibility" }).click();
  // Keep WebSocket delivery real while advancing browser timers beyond the
  // server's idle-detach window. Hidden data must not be decoded.
  await page.clock.install();
  original.send(Buffer.alloc(1));
  await page.clock.runFor(31_000);
  expect(counts.get("primary")).toBe(attached);
  await page.getByRole("button", { name: "Toggle visibility" }).click();
  await expect.poll(() => counts.get("primary")).toBe(attached + 1);
  await expect.poll(() => pixels(page)).toEqual(red);
  sockets.get("primary").send(frame(2, false, [0, 255, 0, 255]));
  await expect
    .poll(() => pixels(page))
    .toEqual([255, 0, 0, 255, 0, 255, 0, 255]);
  await page.getByRole("button", { name: "Move pane" }).click();
  await expect.poll(() => counts.get("primary")).toBe(attached + 3); // StrictMode mounts the moved pane twice.
  await expect.poll(() => pixels(page)).toEqual(red);
  expect(errors).toEqual([]);
});

test("WebGL context loss replaces the canvas and recovers through the real 2D renderer", async ({
  page,
}) => {
  const { counts, errors } = await streams(page);
  await page.goto("/e2e/fixtures/vnc.html");
  const canvas = page.getByTestId("primary").locator("canvas");
  await expect(canvas).toHaveAttribute("width", "2");
  const supportsLoss = await canvas.evaluate((element) => {
    const gl = element.getContext("webgl2");
    const extension = gl?.getExtension("WEBGL_lose_context");
    if (!extension) return false;
    element.dataset.original = "true";
    extension.loseContext();
    return true;
  });
  test.skip(!supportsLoss, "Browser has no WebGL context-loss extension");
  await expect(canvas).not.toHaveAttribute("data-original");
  await expect.poll(() => pixels(page)).toEqual(red);
  expect(counts.get("primary")).toBeGreaterThan(1);
  expect(errors).toEqual([]);
});
