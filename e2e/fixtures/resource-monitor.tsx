import { createInstance } from "i18next";
import { createRoot } from "react-dom/client";
import { I18nextProvider } from "react-i18next";
import ResourceMonitor from "../../src/components/panel/ResourceMonitor";
import type { NetworkHistorySeries } from "../../src/hooks/useNetworkHistory";
import en from "../../src/i18n/locales/en.json";
import type { RemoteStats } from "../../src/types/global";
import "../../src/index.css";

const params = new URLSearchParams(location.search);
const cores = Number(params.get("cores") ?? 32);
document.documentElement.style.fontSize = `${Number(params.get("fontSize") ?? 16)}px`;

const i18n = createInstance();
await i18n.init({ lng: "en", resources: { en: { translation: en } } });

const stats: RemoteStats = {
  system: {
    hostname: "layout-fixture",
    uptime_sec: 100,
    os: "Linux",
    arch: "x86_64",
  },
  load: { load1: 0.1, load5: 0.2, load15: 0.3 },
  cpu: {
    model: "CPU",
    cores,
    usage: 10,
    per_core: Array.from({ length: cores }, (_, id) => ({
      id,
      usage: id % 100,
    })),
    sample_window_ms: 1000,
    usage_source: "aggregate",
  },
  memory: { used: 1024, available: 1024, cached: 0 },
  networks: [],
  network_summary: { rx_bytes_per_sec: 0, tx_bytes_per_sec: 0 },
  disks: [],
};
const history: NetworkHistorySeries = { summary: [], interfaces: {} };

createRoot(document.getElementById("root")!).render(
  <I18nextProvider i18n={i18n}>
    <ResourceMonitor
      activeSessionId="layout-fixture"
      enabled
      remoteStats={{
        sessionId: "layout-fixture",
        stats,
        error: false,
        isManualRefreshing: false,
        refresh: () => {},
      }}
      networkHistoryStore={{
        getSeries: () => history,
        subscribe: () => () => {},
      }}
    />
  </I18nextProvider>,
);
