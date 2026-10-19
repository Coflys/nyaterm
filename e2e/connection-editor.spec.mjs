import { expect, test } from "@playwright/test";

const categories = [
  "Basic information",
  "Authentication",
  "Network & proxy",
  "Terminal",
  "SFTP",
  "Forwarding",
  "Post-login commands",
  "Security & algorithms",
];
async function navigate(page, name) {
  const dropdown = page.getByRole("combobox", {
    name: "Configuration category",
  });
  if (await dropdown.count()) {
    await dropdown.click();
    await page.getByRole("option", { name, exact: true }).click();
  } else await page.getByRole("tab", { name, exact: true }).click();
}

for (const [width, height] of [
  [900, 680],
  [760, 680],
  [759, 680],
  [420, 480],
]) {
  test(`six protocol fields fit ${width} × ${height}, with fixed actions and independent scrolling`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height });
    for (const protocol of ["ssh", "telnet", "rdp", "vnc", "serial", "local"]) {
      await page.goto(
        `/e2e/fixtures/connection-editor.html?edit=fixture&protocol=${protocol}`,
        { waitUntil: "domcontentloaded" },
      );
      await expect(
        page.getByRole("button", { name: "Save", exact: true }),
      ).toBeEnabled();
      const dropdown = page.getByRole("combobox", {
        name: "Configuration category",
      });
      await expect(dropdown).toHaveCount(width >= 760 ? 0 : 1);
      const names =
        protocol === "ssh"
          ? categories
          : protocol === "telnet"
            ? categories.slice(0, 4).concat("Protocol & compatibility")
            : protocol === "serial"
              ? ["Basic information", "Serial port", "Terminal & transfer"]
              : protocol === "local"
                ? ["Basic information", "Startup", "Terminal"]
                : [
                    "Basic information",
                    "Authentication",
                    "Network & proxy",
                    "Security",
                    "Display",
                    "Session",
                  ];
      for (const name of names) {
        await navigate(page, name);
        const content = page.locator("#connection-editor-content");
        await expect(
          page.getByRole("heading", { name, exact: true }),
        ).toBeVisible();
        expect(
          await content.evaluate(
            (element) => element.scrollWidth - element.clientWidth,
          ),
        ).toBeLessThanOrEqual(1);
        expect(
          await page.evaluate(() => document.documentElement.scrollHeight),
        ).toBe(height);
        await expect(
          page.getByRole("button", { name: "Save", exact: true }),
        ).toBeInViewport({ ratio: 1 });
      }
    }
  });
}

test("large fonts and long translations preserve navigation and algorithm scrolling", async ({
  page,
}) => {
  await page.setViewportSize({ width: 760, height: 480 });
  await page.goto(
    "/e2e/fixtures/connection-editor.html?edit=fixture&fontSize=24&lang=ru",
    { waitUntil: "domcontentloaded" },
  );
  await page
    .getByRole("tab", { name: "Безопасность и алгоритмы", exact: true })
    .click();
  const content = page.locator("#connection-editor-content");
  expect(
    await content.evaluate(
      (element) => element.scrollWidth - element.clientWidth,
    ),
  ).toBeLessThanOrEqual(1);
  await expect(
    page.getByRole("button", { name: "Сохранить", exact: true }),
  ).toBeInViewport({ ratio: 1 });
  await page.screenshot({
    path: "temp/connection-editor-results/large-font.png",
  });
});

test("resizing preserves the field and keyboard navigation focus", async ({
  page,
}) => {
  await page.goto("/e2e/fixtures/connection-editor.html?edit=fixture", {
    waitUntil: "domcontentloaded",
  });
  const host = page.locator('[data-editor-field="host"]');
  await host.fill("edited.example.org");
  await page.getByRole("tab", { name: "Authentication", exact: true }).click();
  await page.setViewportSize({ width: 759, height: 680 });
  await expect(
    page.getByRole("combobox", { name: "Configuration category" }),
  ).toBeFocused();
  await navigate(page, "Basic information");
  await expect(host).toHaveValue("edited.example.org");
});

test("the production Web iframe host sizes to the available space", async ({
  page,
}) => {
  await page.goto("/e2e/fixtures/connection-editor.html?web=1&host=1", {
    waitUntil: "domcontentloaded",
  });
  const editor = page.frameLocator("iframe");
  await expect(
    editor.getByRole("button", { name: "Save", exact: true }),
  ).toBeEnabled();
  await page.setViewportSize({ width: 440, height: 500 });
  await expect(
    editor.getByRole("combobox", { name: "Configuration category" }),
  ).toBeVisible();
  await expect(
    editor.getByRole("button", { name: "Save", exact: true }),
  ).toBeInViewport({ ratio: 1 });
});
