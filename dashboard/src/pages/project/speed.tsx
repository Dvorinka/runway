import {
  api,
  projectEventsStream,
  type CronJob,
  type Deployment,
  type Domain,
  type EnvVar,
  type Project,
  type RedirectRule,
  type RemoteNode,
  type Webhook,
} from "@/lib/api";
import { Avatar, AvatarRow, Badge, Button, Card, CardFooter, ComboBox, EmptyState, Input, SectionHead, Skeleton, StatusDot, displayStatus } from "@/components/ui";
import { DeploymentRow } from "@/components/DeploymentRow";
import { filesToTarGz } from "@/lib/tarball";
import { cn, timeAgo } from "@/lib/utils";
import { Globe, Link2, Rocket, Webhook as WebhookIcon, ArrowRightLeft, ListFilter, CheckCircle2, Circle, KeyRound, Clock } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import { DOCS, Err, inputCls } from "./shared";

export const VITALS: {
  key: "lcp" | "fcp" | "inp" | "cls" | "ttfb";
  label: string;
  good: number;
  poor: number;
  fmt: (v: number) => string;
}[] = [
  { key: "lcp", label: "LCP", good: 2500, poor: 4000, fmt: (v) => (v / 1000).toFixed(2) + " s" },
  { key: "inp", label: "INP", good: 200, poor: 500, fmt: (v) => Math.round(v) + " ms" },
  { key: "cls", label: "CLS", good: 0.1, poor: 0.25, fmt: (v) => v.toFixed(2) },
  { key: "fcp", label: "FCP", good: 1800, poor: 3000, fmt: (v) => (v / 1000).toFixed(2) + " s" },
  { key: "ttfb", label: "TTFB", good: 800, poor: 1800, fmt: (v) => Math.round(v) + " ms" },
];

export function Speed({ id }: { id: string }) {
  const [enabled, setEnabled] = useState(false);
  const [data, setData] = useState<Awaited<ReturnType<typeof api.speedInsights>> | null>(null);
  const [error, setError] = useState("");
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    api
      .project(id)
      .then((p) => {
        const s = (p.config?.speed_insights ?? {}) as { enabled?: boolean };
        setEnabled(!!s.enabled);
      })
      .catch(() => {});
    api
      .speedInsights(id)
      .then(setData)
      .catch((e) => setError(e instanceof Error ? e.message : "failed to load"))
      .finally(() => setLoaded(true));
  }, [id]);

  async function toggle() {
    const next = !enabled;
    setEnabled(next);
    setError("");
    try {
      await api.patchProject(id, { config: { speed_insights: { enabled: next } } });
    } catch (e) {
      setEnabled(!next);
      setError(e instanceof Error ? e.message : "save failed");
    }
  }

  if (!loaded) return <Skeleton className="h-40" />;

  return (
    <div className="max-w-2xl space-y-4">
      <Card className="p-5">
        <div className="flex items-start justify-between gap-4">
          <div>
            <h2 className="mb-1 text-sm font-medium">Speed insights</h2>
            <p className="text-xs text-muted-foreground">
              Real-user Core Web Vitals, collected by a ~1 KB script injected into every HTML
              page on the next static deploy. Data stays on your instance — nothing leaves.
            </p>
          </div>
          <button
            onClick={toggle}
            role="switch"
            aria-checked={enabled}
            aria-label="Enable speed insights"
            className={`relative h-5 w-9 shrink-0 rounded-full transition-colors ${
              enabled ? "bg-emerald-500" : "bg-muted"
            }`}
          >
            <span
              className={`absolute top-0.5 h-4 w-4 rounded-full bg-white transition-all ${
                enabled ? "left-[18px]" : "left-0.5"
              }`}
            />
          </button>
        </div>
        <Err msg={error} />
      </Card>

      {enabled && (data?.views ?? 0) === 0 && (
        <Card className="p-5 text-sm text-muted-foreground">
          No data yet — redeploy to inject the beacon, then visits report automatically.
        </Card>
      )}

      {enabled && (data?.views ?? 0) > 0 && (
        <>
          <div className="grid grid-cols-2 gap-3 sm:grid-cols-5">
            {VITALS.map((v) => {
              const val = data!.p75[v.key];
              const cls =
                val == null
                  ? "text-muted-foreground"
                  : val <= v.good
                    ? "text-emerald-400"
                    : val <= v.poor
                      ? "text-amber-400"
                      : "text-red-400";
              return (
                <Card key={v.key} className="p-3">
                  <p className="text-[11px] uppercase tracking-wide text-muted-foreground">{v.label}</p>
                  <p className={`mt-1 font-mono text-lg ${cls}`}>{val == null ? "—" : v.fmt(val)}</p>
                </Card>
              );
            })}
          </div>
          <Card className="p-5">
            <h3 className="mb-3 text-sm font-medium">Top paths · p75 LCP</h3>
            <div className="space-y-2">
              {data!.paths.map((p) => (
                <div key={p.path} className="flex items-center justify-between text-sm">
                  <span className="truncate font-mono text-xs">{p.path}</span>
                  <span className="shrink-0 text-xs text-muted-foreground">
                    {p.lcp != null ? (p.lcp / 1000).toFixed(2) + " s" : "—"} · {p.views} views
                  </span>
                </div>
              ))}
            </div>
          </Card>
        </>
      )}
    </div>
  );
}
