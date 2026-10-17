import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { FaMemory } from "react-icons/fa6";
import { LuCpu } from "react-icons/lu";
import {
  MdComputer,
  MdExpandMore,
  MdMonitorHeart,
  MdRefresh,
  MdStorage,
  MdSwapVert,
} from "react-icons/md";
import PanelHeader from "@/components/layout/PanelHeader";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { type NetworkHistoryStore, useNetworkHistorySeries } from "@/hooks/useNetworkHistory";
import type { RemoteStatsState } from "@/hooks/useRemoteStats";
import NetworkTrafficChart, { formatNetworkRate } from "./NetworkTrafficChart";

const ALL_NETWORK_INTERFACES = "__all__";

function formatBytes(bytes: number): string {
  if (bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const val = bytes / 1024 ** i;
  return `${val < 10 ? val.toFixed(2) : val < 100 ? val.toFixed(1) : val.toFixed(0)} ${units[i]}`;
}

function formatUptime(
  seconds: number,
  t: (key: string, opts?: Record<string, unknown>) => string,
): string {
  const days = Math.floor(seconds / 86400);
  return t(days === 1 ? "resourceMonitor.day" : "resourceMonitor.days", { count: days });
}

function formatPct(value: number | null): string {
  if (value == null) return "--";
  return `${Math.round(Math.min(100, Math.max(0, value)))}%`;
}

function usageColor(pct: number | null): string {
  if (pct == null) return "var(--df-text-dimmed)";
  if (pct >= 90) return "#ef4444";
  if (pct >= 70) return "#f59e0b";
  return "var(--df-primary)";
}

function ProgressBar({ value, color }: { value: number | null; color: string }) {
  const pct = value == null ? 0 : Math.min(100, Math.max(0, value));
  const isHigh = pct >= 80;
  return (
    <div
      className="relative h-1.5 w-full rounded-full overflow-hidden"
      style={{ backgroundColor: "color-mix(in srgb, var(--df-border) 60%, transparent)" }}
    >
      <div
        className="absolute inset-y-0 left-0 rounded-full transition-all duration-700 ease-out"
        style={{
          width: `${pct}%`,
          background: isHigh ? `linear-gradient(90deg, ${color}cc, ${color})` : color,
          boxShadow: isHigh ? `0 0 8px ${color}66` : "none",
        }}
      />
    </div>
  );
}

function RingGauge({
  value,
  color,
  size = 56,
  strokeWidth = 5,
}: {
  value: number | null;
  color: string;
  size?: number;
  strokeWidth?: number;
}) {
  const pct = value == null ? 0 : Math.min(100, Math.max(0, value));
  const radius = (size - strokeWidth) / 2;
  const circumference = 2 * Math.PI * radius;
  const dashOffset = circumference * (1 - pct / 100);

  return (
    <div className="relative shrink-0" style={{ width: size, height: size }}>
      <svg width={size} height={size} className="rotate-[-90deg]" aria-hidden="true">
        <circle
          cx={size / 2}
          cy={size / 2}
          r={radius}
          fill="none"
          stroke="color-mix(in srgb, var(--df-border) 60%, transparent)"
          strokeWidth={strokeWidth}
        />
        <circle
          cx={size / 2}
          cy={size / 2}
          r={radius}
          fill="none"
          stroke={color}
          strokeWidth={strokeWidth}
          strokeLinecap="round"
          strokeDasharray={circumference}
          strokeDashoffset={dashOffset}
          className="transition-all duration-700 ease-out"
        />
      </svg>
      <div className="absolute inset-0 flex items-center justify-center">
        <span className="text-xs font-bold font-mono tabular-nums leading-none" style={{ color }}>
          {value == null ? (
            "--"
          ) : (
            <>
              {Math.round(pct)}
              <span className="text-[0.5625rem] font-normal">%</span>
            </>
          )}
        </span>
      </div>
    </div>
  );
}

function SectionCard({
  icon,
  title,
  children,
  accent,
  action,
}: {
  icon: React.ReactNode;
  title: string;
  children: React.ReactNode;
  accent?: boolean;
  action?: React.ReactNode;
}) {
  return (
    <div
      className="nyaterm-wallpaper-card rounded-lg border px-3 py-2.5 space-y-2 transition-colors duration-200"
      style={{
        borderColor: accent
          ? "color-mix(in srgb, var(--df-primary) 30%, var(--df-border))"
          : "var(--df-border)",
        backgroundColor: accent
          ? "color-mix(in srgb, var(--df-primary) 4%, var(--df-bg))"
          : "var(--df-bg)",
      }}
    >
      <div className="flex items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-1.5">
          <span
            className="text-sm"
            style={{ color: accent ? "var(--df-primary)" : "var(--df-text-muted)" }}
          >
            {icon}
          </span>
          <span
            className="truncate text-xs font-semibold tracking-wide"
            style={{ color: "var(--df-text)" }}
          >
            {title}
          </span>
        </div>
        {action}
      </div>
      {children}
    </div>
  );
}

interface ResourceMonitorProps {
  activeSessionId: string | null;
  enabled: boolean;
  remoteStats: RemoteStatsState;
  networkHistoryStore: NetworkHistoryStore;
}

export default function ResourceMonitor({
  activeSessionId,
  enabled,
  remoteStats,
  networkHistoryStore,
}: ResourceMonitorProps) {
  const { t } = useTranslation();
  const networkHistory = useNetworkHistorySeries(networkHistoryStore, activeSessionId);
  const [cpuExpanded, setCpuExpanded] = useState(false);
  const [selectedNetworkInterfaces, setSelectedNetworkInterfaces] = useState<
    Record<string, string>
  >({});
  const { stats, error, isManualRefreshing, refresh } = remoteStats;

  const selectedNetworkInterface = activeSessionId
    ? (selectedNetworkInterfaces[activeSessionId] ?? ALL_NETWORK_INTERFACES)
    : ALL_NETWORK_INTERFACES;
  const selectedNetwork =
    selectedNetworkInterface === ALL_NETWORK_INTERFACES
      ? null
      : (stats?.networks.find((network) => network.nic === selectedNetworkInterface) ?? null);
  const effectiveNetworkInterface =
    selectedNetworkInterface === ALL_NETWORK_INTERFACES || selectedNetwork
      ? selectedNetworkInterface
      : ALL_NETWORK_INTERFACES;

  useEffect(() => {
    if (
      !activeSessionId ||
      !stats ||
      selectedNetworkInterface === ALL_NETWORK_INTERFACES ||
      selectedNetwork
    ) {
      return;
    }
    setSelectedNetworkInterfaces((current) => ({
      ...current,
      [activeSessionId]: ALL_NETWORK_INTERFACES,
    }));
  }, [activeSessionId, selectedNetwork, selectedNetworkInterface, stats]);

  const currentTraffic =
    effectiveNetworkInterface === ALL_NETWORK_INTERFACES ? stats?.network_summary : selectedNetwork;
  const networkHistoryPoints =
    effectiveNetworkInterface === ALL_NETWORK_INTERFACES
      ? networkHistory.summary
      : (networkHistory.interfaces[effectiveNetworkInterface] ?? []);
  const networkHistoryMinutes =
    networkHistoryPoints.length >= 2
      ? Math.max(
          0.1,
          Math.round(
            ((networkHistoryPoints[networkHistoryPoints.length - 1]?.timestamp ?? 0) -
              networkHistoryPoints[0].timestamp) /
              6000,
          ) / 10,
        )
      : null;

  const memTotal = stats ? stats.memory.used + stats.memory.available : 0;
  const memUsedPct = memTotal > 0 ? (stats!.memory.used / memTotal) * 100 : 0;
  const cpuUsage = stats?.cpu.usage ?? null;
  return (
    <div
      className="nyaterm-wallpaper-transparent-surface h-full flex flex-col"
      style={{ backgroundColor: "var(--df-bg-panel)" }}
    >
      <PanelHeader
        title={t("panel.resourceMonitor")}
        actions={
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                className="h-7 w-7 rounded-md text-muted-foreground hover:text-foreground disabled:opacity-40"
                onClick={refresh}
                disabled={!enabled || !activeSessionId || isManualRefreshing}
                aria-label={t("resourceMonitor.refresh")}
              >
                <MdRefresh className={`h-4 w-4 ${isManualRefreshing ? "animate-spin" : ""}`} />
              </Button>
            </TooltipTrigger>
            <TooltipContent side="top">{t("resourceMonitor.refresh")}</TooltipContent>
          </Tooltip>
        }
      />

      <div className="flex-1 overflow-y-auto p-2.5 terminal-scroll">
        {!activeSessionId ? (
          <EmptyState icon={<MdMonitorHeart />} text={t("panel.resourceMonitorNoSession")} />
        ) : !enabled ? (
          <EmptyState icon={<MdMonitorHeart />} text={t("panel.resourceMonitorDisabled")} />
        ) : error && !stats ? (
          <EmptyState icon={<MdMonitorHeart />} text={t("panel.resourceMonitorError")} />
        ) : stats ? (
          <div className="space-y-2">
            {/* System */}
            <SectionCard icon={<MdComputer />} title={t("resourceMonitor.system")}>
              <div
                className="grid gap-x-4 gap-y-1"
                style={{ gridTemplateColumns: "minmax(0, 1fr) max-content" }}
              >
                <InfoCell label={t("resourceMonitor.hostname")} value={stats.system.hostname} />
                <InfoCell label={t("resourceMonitor.arch")} value={stats.system.arch} />
                <InfoCell label={t("resourceMonitor.os")} value={stats.system.os} />
                <InfoCell
                  label={t("resourceMonitor.uptime")}
                  value={formatUptime(stats.system.uptime_sec, t)}
                />
              </div>
            </SectionCard>

            {/* CPU + Load */}
            <SectionCard icon={<LuCpu />} title={t("resourceMonitor.cpu")}>
              <div className="space-y-2.5">
                {/* Ring gauge + average usage */}
                <div className="flex items-center gap-3">
                  <RingGauge
                    value={cpuUsage}
                    color={usageColor(cpuUsage)}
                    size={56}
                    strokeWidth={5}
                  />
                  <div className="flex-1 min-w-0 space-y-1">
                    <div className="flex items-baseline justify-between">
                      <span className="text-[0.6875rem]" style={{ color: "var(--df-text-muted)" }}>
                        {t("resourceMonitor.cpuAvgUsage")}
                      </span>
                      <span
                        className="text-sm font-bold font-mono tabular-nums"
                        style={{ color: usageColor(cpuUsage) }}
                      >
                        {cpuUsage == null ? "--" : `${cpuUsage.toFixed(1)}%`}
                      </span>
                    </div>
                    <ProgressBar value={cpuUsage} color={usageColor(cpuUsage)} />
                    <div className="text-right">
                      <span
                        className="text-[0.625rem] font-mono"
                        style={{ color: "var(--df-text-dimmed)" }}
                      >
                        {cpuUsage == null ? t("resourceMonitor.sampling") : `${stats.cpu.cores}C`}
                      </span>
                    </div>
                  </div>
                </div>

                {/* Load badges */}
                <div className="grid grid-cols-3 gap-1.5">
                  <LoadBadge label={t("resourceMonitor.Load1")} value={stats.load.load1} />
                  <LoadBadge label={t("resourceMonitor.Load5")} value={stats.load.load5} />
                  <LoadBadge label={t("resourceMonitor.Load15")} value={stats.load.load15} />
                </div>

                {/* Expand toggle for per-core */}
                {stats.cpu.per_core.length > 0 && (
                  <div>
                    <button
                      type="button"
                      className="flex items-center gap-0.5 text-[0.6875rem] transition-colors duration-150 rounded px-1 py-0.5 -ml-1"
                      style={{ color: "var(--df-text-muted)" }}
                      aria-expanded={cpuExpanded}
                      onClick={() => setCpuExpanded((v) => !v)}
                      onMouseEnter={(e) => {
                        e.currentTarget.style.backgroundColor =
                          "color-mix(in srgb, var(--df-border) 50%, transparent)";
                      }}
                      onMouseLeave={(e) => {
                        e.currentTarget.style.backgroundColor = "transparent";
                      }}
                    >
                      <MdExpandMore
                        className="transition-transform duration-200"
                        style={{
                          transform: cpuExpanded ? "rotate(180deg)" : "rotate(0deg)",
                        }}
                      />
                      <span>
                        {stats.cpu.per_core.length} {t("resourceMonitor.cpu")}
                      </span>
                    </button>

                    <div
                      className="grid transition-[grid-template-rows,opacity] duration-300 ease-in-out"
                      aria-hidden={!cpuExpanded}
                      style={{
                        gridTemplateRows: cpuExpanded ? "1fr" : "0fr",
                        opacity: cpuExpanded ? 1 : 0,
                      }}
                    >
                      <div className="min-h-0 overflow-hidden">
                        <div className="pt-1.5 space-y-0.5">
                          {stats.cpu.per_core.map((core) => {
                            return (
                              <CoreRow
                                key={`cpu-core-${core.id}`}
                                id={core.id}
                                usage={core.usage}
                              />
                            );
                          })}
                        </div>
                      </div>
                    </div>
                  </div>
                )}
              </div>
            </SectionCard>

            {/* Memory */}
            <SectionCard icon={<FaMemory />} title={t("resourceMonitor.memory")}>
              <div className="space-y-2">
                <div className="flex items-center gap-3">
                  <RingGauge
                    value={memUsedPct}
                    color={usageColor(memUsedPct)}
                    size={56}
                    strokeWidth={5}
                  />
                  <div className="flex-1 min-w-0 space-y-1">
                    <div className="flex items-baseline justify-between">
                      <span className="text-[0.6875rem]" style={{ color: "var(--df-text-muted)" }}>
                        RAM
                      </span>
                      <span
                        className="text-sm font-bold font-mono tabular-nums"
                        style={{ color: usageColor(memUsedPct) }}
                      >
                        {formatPct(memUsedPct)}
                      </span>
                    </div>
                    <ProgressBar value={memUsedPct} color={usageColor(memUsedPct)} />
                    <div
                      className="text-[0.625rem] font-mono tabular-nums"
                      style={{ color: "var(--df-text-muted)" }}
                    >
                      {formatBytes(stats.memory.used)}
                      <span style={{ color: "var(--df-text-dimmed)" }}> / </span>
                      {formatBytes(memTotal)}
                    </div>
                  </div>
                </div>
                <div className="flex flex-wrap gap-x-3 gap-y-0.5">
                  <MetricChip
                    label={t("resourceMonitor.available")}
                    value={formatBytes(stats.memory.available)}
                  />
                  <MetricChip
                    label={t("resourceMonitor.cached")}
                    value={formatBytes(stats.memory.cached)}
                  />
                </div>
              </div>
            </SectionCard>

            {/* Network */}
            <SectionCard
              icon={<MdSwapVert />}
              title={t("resourceMonitor.network")}
              action={
                stats.networks.length > 0 ? (
                  <Select
                    value={effectiveNetworkInterface}
                    onValueChange={(value) => {
                      if (!activeSessionId) return;
                      setSelectedNetworkInterfaces((current) => ({
                        ...current,
                        [activeSessionId]: value,
                      }));
                    }}
                  >
                    <SelectTrigger
                      size="sm"
                      className="h-6 max-w-[9rem] border-0 px-1.5 text-[0.6875rem] shadow-none"
                      aria-label={t("resourceMonitor.nic")}
                    >
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent align="end">
                      <SelectItem value={ALL_NETWORK_INTERFACES}>
                        {t("resourceMonitor.allInterfaces")}
                      </SelectItem>
                      {stats.networks.map((network) => (
                        <SelectItem key={network.nic} value={network.nic}>
                          {network.nic}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                ) : undefined
              }
            >
              {stats.networks.length > 0 ? (
                <div className="space-y-2.5">
                  <div className="grid grid-cols-2 gap-2">
                    <NetworkMetric
                      label={t("resourceMonitor.download")}
                      value={formatNetworkRate(currentTraffic?.rx_bytes_per_sec ?? 0)}
                      arrow="↓"
                      color="#3b82f6"
                    />
                    <NetworkMetric
                      label={t("resourceMonitor.upload")}
                      value={formatNetworkRate(currentTraffic?.tx_bytes_per_sec ?? 0)}
                      arrow="↑"
                      color="#22c55e"
                    />
                  </div>
                  <NetworkTrafficChart points={networkHistoryPoints} />
                  {networkHistoryMinutes != null && (
                    <div
                      className="text-right text-[0.625rem]"
                      style={{ color: "var(--df-text-dimmed)" }}
                    >
                      {t("resourceMonitor.recentMinutes", { count: networkHistoryMinutes })}
                    </div>
                  )}
                </div>
              ) : (
                <span className="text-xs" style={{ color: "var(--df-text-dimmed)" }}>
                  -
                </span>
              )}
            </SectionCard>

            {/* Disk */}
            <SectionCard icon={<MdStorage />} title={t("resourceMonitor.disk")}>
              {stats.disks.length > 0 ? (
                <div className="space-y-0">
                  {stats.disks.map((disk) => (
                    <DiskRow
                      key={`${disk.device}-${disk.mount}`}
                      mount={disk.mount}
                      total={disk.total}
                      available={disk.available}
                      availableLabel={t("resourceMonitor.available")}
                      usePercent={disk.use_percent}
                    />
                  ))}
                </div>
              ) : (
                <span className="text-xs" style={{ color: "var(--df-text-dimmed)" }}>
                  -
                </span>
              )}
            </SectionCard>
          </div>
        ) : (
          <LoadingSpinner label={t("common.loading")} />
        )}
      </div>
    </div>
  );
}

function EmptyState({ icon, text }: { icon: React.ReactNode; text: string }) {
  return (
    <div className="flex flex-col items-center justify-center h-full text-center gap-2 px-4">
      <span className="text-2xl" style={{ color: "var(--df-text-dimmed)" }}>
        {icon}
      </span>
      <span className="text-sm" style={{ color: "var(--df-text-muted)" }}>
        {text}
      </span>
    </div>
  );
}

function LoadingSpinner({ label }: { label: string }) {
  return (
    <div className="flex items-center justify-center h-full">
      <svg
        className="animate-spin w-5 h-5"
        style={{ color: "var(--df-text-dimmed)" }}
        xmlns="http://www.w3.org/2000/svg"
        fill="none"
        viewBox="0 0 24 24"
      >
        <title>{label}</title>
        <circle
          className="opacity-25"
          cx="12"
          cy="12"
          r="10"
          stroke="currentColor"
          strokeWidth="4"
        />
        <path
          className="opacity-75"
          fill="currentColor"
          d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
        />
      </svg>
    </div>
  );
}

function InfoCell({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0">
      <div className="text-[0.625rem]" style={{ color: "var(--df-text-dimmed)" }}>
        {label}
      </div>
      <div
        className="text-xs font-semibold font-mono truncate"
        style={{ color: "var(--df-text)" }}
        title={value}
      >
        {value}
      </div>
    </div>
  );
}

function LoadBadge({ label, value }: { label: string; value: number }) {
  return (
    <div
      className="min-w-0 rounded-md border px-2 py-1.5 text-center"
      style={{
        backgroundColor: "color-mix(in srgb, var(--df-border) 18%, var(--df-bg))",
        borderColor: "color-mix(in srgb, var(--df-border) 75%, transparent)",
      }}
    >
      <div
        className="truncate text-xs font-bold font-mono tabular-nums leading-none"
        style={{ color: "var(--df-text)" }}
        title={value.toFixed(2)}
      >
        {value.toFixed(2)}
      </div>
      <div
        className="mt-1 truncate text-[0.5625rem] font-medium leading-none"
        style={{ color: "var(--df-text-dimmed)" }}
        title={label}
      >
        {label}
      </div>
    </div>
  );
}

function CoreRow({ id, usage }: { id: number; usage: number }) {
  const color = usageColor(usage);
  return (
    <div className="flex items-center gap-1.5 min-h-[22px]">
      <span
        className="w-8 text-right text-[0.625rem] font-mono tabular-nums shrink-0"
        style={{ color: "var(--df-text-muted)" }}
      >
        cpu{id}
      </span>
      <span
        className="inline-block w-1.5 h-1.5 rounded-full shrink-0"
        style={{ backgroundColor: color, boxShadow: `0 0 4px ${color}66` }}
      />
      <div className="flex-1 min-w-0">
        <ProgressBar value={usage} color={color} />
      </div>
      <span
        className="w-10 text-right text-[0.625rem] font-mono tabular-nums shrink-0"
        style={{ color: "var(--df-text-muted)" }}
      >
        {usage.toFixed(1)}%
      </span>
    </div>
  );
}

function MetricChip({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center gap-1">
      <span className="text-[0.625rem]" style={{ color: "var(--df-text-dimmed)" }}>
        {label}
      </span>
      <span
        className="text-[0.6875rem] font-mono tabular-nums"
        style={{ color: "var(--df-text-muted)" }}
      >
        {value}
      </span>
    </div>
  );
}

function DiskRow({
  mount,
  total,
  available,
  availableLabel,
  usePercent,
}: {
  mount: string;
  total: number;
  available: number;
  availableLabel: string;
  usePercent: number;
}) {
  return (
    <div
      className="space-y-1.5 border-b py-2 first:pt-0 last:border-b-0 last:pb-0"
      style={{ borderColor: "color-mix(in srgb, var(--df-border) 60%, transparent)" }}
    >
      <div className="flex items-baseline justify-between gap-2">
        <span
          className="min-w-0 truncate text-xs font-mono font-medium"
          style={{ color: "var(--df-text)" }}
          title={mount}
        >
          {mount}
        </span>
        <span
          className="shrink-0 text-xs font-mono font-bold tabular-nums"
          style={{ color: usageColor(usePercent) }}
        >
          {formatPct(usePercent)}
        </span>
      </div>
      <ProgressBar value={usePercent} color={usageColor(usePercent)} />
      <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5">
        <span className="text-[0.625rem] font-mono" style={{ color: "var(--df-text-dimmed)" }}>
          {formatBytes(total)}
        </span>
        <MetricChip label={availableLabel} value={formatBytes(available)} />
      </div>
    </div>
  );
}

function NetworkMetric({
  label,
  value,
  arrow,
  color,
}: {
  label: string;
  value: string;
  arrow: string;
  color: string;
}) {
  return (
    <div
      className="min-w-0 rounded-md border px-2 py-1.5"
      style={{
        backgroundColor: "color-mix(in srgb, var(--df-border) 18%, var(--df-bg))",
        borderColor: "color-mix(in srgb, var(--df-border) 70%, transparent)",
      }}
    >
      <div className="text-[0.625rem]" style={{ color: "var(--df-text-dimmed)" }}>
        <span className="mr-1" style={{ color }}>
          {arrow}
        </span>
        {label}
      </div>
      <div
        className="mt-0.5 truncate text-xs font-bold font-mono tabular-nums"
        style={{ color: "var(--df-text)" }}
        title={value}
      >
        {value}
      </div>
    </div>
  );
}
