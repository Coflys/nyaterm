import { StrictMode, useState } from "react";
import { createRoot } from "react-dom/client";
import { createInstance } from "i18next";
import { I18nextProvider } from "react-i18next";
import VncPaneHost from "../../src/components/vnc/VncPaneHost";
import type { VncSessionPane } from "../../src/types/global";
import en from "../../src/i18n/locales/en.json";

const i18n = createInstance();
await i18n.init({ lng: "en", resources: { en: { translation: en } } });
const pane = (id: string): VncSessionPane => ({
  kind: "leaf",
  paneKind: "remote-desktop",
  type: "VNC",
  id,
  sessionId: id,
  name: id,
  connectionId: "fixture",
  display: { scaleMode: "fit", viewOnly: true, clipboardEnabled: false },
});
function Fixture() {
  const [visible, setVisible] = useState(true);
  const [moved, setMoved] = useState(false);
  return (
    <>
      <button onClick={() => setVisible((value) => !value)}>
        Toggle visibility
      </button>
      <button onClick={() => setMoved((value) => !value)}>Move pane</button>
      <div
        style={{
          display: visible ? "flex" : "none",
          height: 200,
          flexDirection: moved ? "column" : "row",
        }}
      >
        <div
          data-testid="primary"
          key={String(moved)}
          style={{ width: 200, height: 100 }}
        >
          <VncPaneHost pane={pane("primary")} active visible={visible} />
        </div>
        <div data-testid="secondary" style={{ width: 200, height: 100 }}>
          <VncPaneHost
            pane={pane("secondary")}
            active={false}
            visible={visible}
          />
        </div>
      </div>
    </>
  );
}
createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <I18nextProvider i18n={i18n}>
      <Fixture />
    </I18nextProvider>
  </StrictMode>,
);
