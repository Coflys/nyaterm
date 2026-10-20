import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ActionButton, ActionFooter } from "@/components/ui/action-footer";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { NumberInput } from "@/components/ui/number-input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { cn } from "@/lib/utils";
import type { NetworkGroup, TunnelConfig } from "@/types/global";
import { ConnectionCombobox, type ConnectionOption } from "./shared";

type TunnelMode = "local" | "remote" | "dynamic";

function getTunnelMode(tunnelType: string): TunnelMode {
  if (tunnelType === "remote" || tunnelType === "dynamic") {
    return tunnelType;
  }
  return "local";
}

function getBindHost(bindLocalhost: boolean) {
  return bindLocalhost ? "127.0.0.1" : "0.0.0.0";
}

export function createTunnelDraft(tunnel?: TunnelConfig | null): TunnelConfig {
  if (tunnel) return { ...tunnel, auto_reconnect: tunnel.auto_reconnect ?? true };
  return {
    id: "",
    name: "",
    tunnel_type: "local",
    connection_id: undefined,
    listen_port: 0,
    target_host: "127.0.0.1",
    target_port: 0,
    is_open: false,
    auto_open: false,
    auto_reconnect: true,
    bind_localhost: true,
    group_id: undefined,
  };
}

function getTunnelFieldCopy(
  tunnelType: string,
  t: (key: string, options?: Record<string, unknown>) => string,
) {
  const tunnelMode = getTunnelMode(tunnelType);

  if (tunnelMode === "remote") {
    return {
      listenPortLabel: t("network.listenPortRemote"),
      listenPortHint: t("network.listenPortRemoteHint"),
      targetHostLabel: t("network.targetHostRemote"),
      targetHostHint: t("network.targetHostRemoteHint"),
      targetPortLabel: t("network.targetPortRemote"),
      targetPortHint: t("network.targetPortRemoteHint"),
    };
  }

  if (tunnelMode === "dynamic") {
    return {
      listenPortLabel: t("network.listenPortDynamic"),
      listenPortHint: t("network.listenPortDynamicHint"),
      targetHostLabel: "",
      targetHostHint: "",
      targetPortLabel: "",
      targetPortHint: "",
    };
  }

  return {
    listenPortLabel: t("network.listenPortLocal"),
    listenPortHint: t("network.listenPortLocalHint"),
    targetHostLabel: t("network.targetHostLocal"),
    targetHostHint: t("network.targetHostLocalHint"),
    targetPortLabel: t("network.targetPortLocal"),
    targetPortHint: t("network.targetPortLocalHint"),
  };
}

function generateTunnelPreview(
  form: TunnelConfig,
  t: (key: string, options?: Record<string, unknown>) => string,
) {
  const tunnelMode = getTunnelMode(form.tunnel_type);
  const bindHost = getBindHost(form.bind_localhost);
  const listenPort = form.listen_port || "?";

  if (tunnelMode === "dynamic") {
    return t("network.tunnelPreviewDynamic", {
      bindHost,
      listenPort,
    });
  }

  const targetHost = form.target_host.trim() || "?";
  const targetPort = form.target_port || "?";

  if (tunnelMode === "remote") {
    return t("network.tunnelPreviewRemote", {
      bindHost,
      listenPort,
      targetHost,
      targetPort,
    });
  }

  return t("network.tunnelPreviewLocal", {
    bindHost,
    listenPort,
    targetHost,
    targetPort,
  });
}

export function TunnelFormContent({
  tunnel,
  connectionOptions,
  groups,
  saving,
  externalError,
  saveDisabled,
  onCancel,
  onSave,
}: {
  tunnel: TunnelConfig | null;
  connectionOptions: ConnectionOption[];
  groups: NetworkGroup[];
  saving: boolean;
  externalError?: string;
  saveDisabled?: boolean;
  onCancel: () => void;
  onSave: (tunnel: TunnelConfig) => Promise<void>;
}) {
  const { t } = useTranslation();
  const [form, setForm] = useState<TunnelConfig>(createTunnelDraft());
  const [error, setError] = useState("");
  const tunnelMode = getTunnelMode(form.tunnel_type);
  const fieldCopy = getTunnelFieldCopy(tunnelMode, t);
  const tunnelPreview = generateTunnelPreview(form, t);

  const updateForm = (patch: Partial<TunnelConfig>) => {
    setForm((prev) => ({ ...prev, ...patch }));
    setError("");
  };

  useEffect(() => {
    setForm(createTunnelDraft(tunnel));
    setError("");
  }, [tunnel]);

  const handleSubmit = async () => {
    const trimmedName = form.name.trim();

    if (!trimmedName) {
      setError(t("network.tunnelNameRequired"));
      return;
    }
    if (!form.connection_id) {
      setError(t("network.connectionRequired"));
      return;
    }
    if (!form.listen_port || form.listen_port < 1 || form.listen_port > 65535) {
      setError(t("network.tunnelListenPortRequired"));
      return;
    }
    if (
      form.tunnel_type !== "dynamic" &&
      (!form.target_host.trim() ||
        !form.target_port ||
        form.target_port < 1 ||
        form.target_port > 65535)
    ) {
      setError(t("network.tunnelTargetRequired"));
      return;
    }

    setError("");
    await onSave({
      ...form,
      name: trimmedName,
      target_host: tunnelMode === "dynamic" ? "127.0.0.1" : form.target_host.trim() || "127.0.0.1",
      target_port: tunnelMode === "dynamic" ? 0 : form.target_port,
      group_id: form.group_id || undefined,
    });
  };

  const displayError = error || externalError || "";

  return (
    <>
      <div className="flex-1 min-h-0 overflow-y-auto p-4 pb-20 sm:p-5 sm:pb-20">
        <p className="mb-4 text-xs leading-5 text-muted-foreground">
          {t("network.tunnelDialogDescription")}
        </p>

        <div className="grid gap-4">
          {/* Tunnel Name, Type and Group */}
          <div className="grid gap-3 sm:grid-cols-[minmax(220px,1fr)_132px_132px]">
            <div className="min-w-0 space-y-1.5">
              <Label className="text-sm">
                {t("network.tunnelName")}
                <span className="ml-1 text-destructive">*</span>
              </Label>
              <Input
                className="h-9 w-full text-sm"
                placeholder={t("network.tunnelNamePlaceholder")}
                value={form.name}
                onChange={(event) => updateForm({ name: event.target.value })}
              />
            </div>

            <div className="min-w-0 space-y-1.5">
              <Label className="text-sm">{t("network.tunnelType")}</Label>
              <Select
                value={tunnelMode}
                onValueChange={(value) => updateForm({ tunnel_type: value as TunnelMode })}
              >
                <SelectTrigger className="h-9 w-full text-sm">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="local">{t("network.localTunnel")}</SelectItem>
                  <SelectItem value="remote">{t("network.remoteTunnel")}</SelectItem>
                  <SelectItem value="dynamic">{t("network.dynamicTunnel")}</SelectItem>
                </SelectContent>
              </Select>
            </div>

            <div className="min-w-0 space-y-1.5">
              <Label className="text-sm">{t("network.group")}</Label>
              <Select
                value={form.group_id || "__ungrouped__"}
                onValueChange={(value) =>
                  updateForm({ group_id: value === "__ungrouped__" ? undefined : value })
                }
              >
                <SelectTrigger className="h-9 w-full text-sm">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="__ungrouped__">{t("network.ungrouped")}</SelectItem>
                  {groups.map((group) => (
                    <SelectItem key={group.id} value={group.id}>
                      {group.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          </div>

          {/* Selected Connection */}
          <div className="space-y-1.5">
            <Label className="text-sm">
              {t("network.selectedConnection")}
              <span className="ml-1 text-destructive">*</span>
            </Label>
            <ConnectionCombobox
              value={form.connection_id ?? ""}
              options={connectionOptions}
              placeholder={t("network.connectionPickerPlaceholder")}
              searchPlaceholder={t("network.searchConnections")}
              emptyText={t("savedConnections.noResults")}
              missingSelectionLabel={t("network.connectionMissing")}
              onChange={(id) => updateForm({ connection_id: id })}
            />
          </div>

          {/* Ports */}
          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-1.5">
              <Label className="text-sm">
                {fieldCopy.listenPortLabel}
                <span className="ml-1 text-destructive">*</span>
              </Label>
              <NumberInput
                className="h-9 text-sm [&_button]:h-9 [&_button]:w-9 [&_input]:h-9 [&_input]:text-sm"
                min={1}
                max={65535}
                value={form.listen_port}
                onChange={(value) => updateForm({ listen_port: value || 0 })}
                required
              />
            </div>
            {tunnelMode !== "dynamic" ? (
              <div className="space-y-1.5">
                <Label className="text-sm">
                  {fieldCopy.targetPortLabel}
                  <span className="ml-1 text-destructive">*</span>
                </Label>
                <NumberInput
                  className="h-9 text-sm [&_button]:h-9 [&_button]:w-9 [&_input]:h-9 [&_input]:text-sm"
                  min={1}
                  max={65535}
                  value={form.target_port}
                  onChange={(value) => updateForm({ target_port: value || 0 })}
                  required
                />
              </div>
            ) : null}
          </div>

          {tunnelMode !== "dynamic" ? (
            <div className="space-y-1.5">
              <Label className="text-sm">{fieldCopy.targetHostLabel}</Label>
              <Input
                className="h-9 text-sm"
                placeholder="127.0.0.1"
                value={form.target_host}
                onChange={(event) => updateForm({ target_host: event.target.value })}
              />
            </div>
          ) : null}

          <div className="space-y-2">
            <div>
              <Label className="text-sm">{t("network.bindAddress")}</Label>
            </div>

            <div
              className="grid gap-2 sm:grid-cols-2"
              role="radiogroup"
              aria-label={t("network.bindAddress")}
            >
              {[
                {
                  value: true,
                  label: t("network.bindLocalhostOnly"),
                  description: t("network.bindLocalhostOnlyHint"),
                },
                {
                  value: false,
                  label: t("network.bindAllInterfaces"),
                  description: t("network.bindAllInterfacesHint"),
                },
              ].map((option) => {
                const checked = form.bind_localhost === option.value;

                return (
                  <label
                    key={option.label}
                    className={cn(
                      "cursor-pointer rounded-md border px-3 py-2 text-left transition-colors",
                      "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50",
                    )}
                    style={{
                      borderColor: checked ? "var(--df-primary)" : "var(--df-border)",
                      backgroundColor: checked
                        ? "color-mix(in srgb, var(--df-bg-hover) 65%, transparent)"
                        : "transparent",
                    }}
                  >
                    <input
                      type="radio"
                      name="bind-address"
                      className="sr-only"
                      checked={checked}
                      onChange={() => updateForm({ bind_localhost: option.value })}
                    />
                    <div className="flex items-center gap-2">
                      <span
                        className="flex size-4 items-center justify-center rounded-full border"
                        style={{
                          borderColor: checked ? "var(--df-primary)" : "var(--df-border)",
                        }}
                      >
                        <span
                          className={cn(
                            "size-2 rounded-full transition-opacity",
                            checked ? "opacity-100" : "opacity-0",
                          )}
                          style={{ backgroundColor: "var(--df-primary)" }}
                        />
                      </span>
                      <span className="text-sm font-medium" style={{ color: "var(--df-text)" }}>
                        {option.label}
                      </span>
                    </div>
                  </label>
                );
              })}
            </div>
          </div>

          <label
            className="flex items-center justify-between rounded-md border px-3 py-2"
            style={{
              borderColor: "var(--df-border)",
              backgroundColor: "color-mix(in srgb, var(--df-bg-hover) 55%, transparent)",
            }}
          >
            <div>
              <div className="text-sm font-medium" style={{ color: "var(--df-text)" }}>
                {t("network.autoOpen")}
              </div>
              <p className="text-xs" style={{ color: "var(--df-text-dimmed)" }}>
                {t("network.tunnelConnectionHint")}
              </p>
            </div>
            <Switch
              checked={form.auto_open}
              onCheckedChange={(checked) => updateForm({ auto_open: checked })}
            />
          </label>

          <label
            className="flex items-center justify-between gap-3 rounded-md border px-3 py-2"
            style={{ borderColor: "var(--df-border)" }}
          >
            <div>
              <div className="text-sm font-medium" style={{ color: "var(--df-text)" }}>
                {t("network.autoReconnect")}
              </div>
              <p className="text-xs" style={{ color: "var(--df-text-dimmed)" }}>
                {t("network.autoReconnectHint")}
              </p>
            </div>
            <Switch
              checked={form.auto_reconnect}
              onCheckedChange={(checked) => updateForm({ auto_reconnect: checked })}
            />
          </label>

          <div
            className="rounded-md border px-3 py-3"
            style={{
              borderColor: "var(--df-border)",
              backgroundColor: "color-mix(in srgb, var(--df-bg-hover) 45%, transparent)",
            }}
          >
            <div
              className="text-[0.6875rem] font-medium"
              style={{ color: "var(--df-text-dimmed)" }}
            >
              {t("network.tunnelPreview")}
            </div>
            <div className="mt-1 font-mono text-sm" style={{ color: "var(--df-text)" }}>
              {tunnelPreview}
            </div>
          </div>

          {displayError ? (
            <div className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive">
              {displayError}
            </div>
          ) : null}
        </div>
      </div>

      <ActionFooter>
        <ActionButton variant="outline" onClick={onCancel} disabled={saving}>
          {t("common.cancel")}
        </ActionButton>
        <ActionButton onClick={handleSubmit} disabled={saving || saveDisabled}>
          {saving ? t("common.saving") : t("common.save")}
        </ActionButton>
      </ActionFooter>
    </>
  );
}
