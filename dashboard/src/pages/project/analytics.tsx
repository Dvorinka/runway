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

export type AnalyticsProvider = "none" | "umami" | "rybbit" | "plausible" | "custom";

export function buildSnippet(p: AnalyticsProvider, fields: { siteId: string; src: string; domain: string; custom: string }): string {
  switch (p) {
    case "umami":
      return `<script defer src="${fields.src || "https://cloud.umami.is/script.js"}" data-website-id="${fields.siteId}"></script>`;
    case "rybbit":
      return `<script defer src="${fields.src || "https://app.rybbit.io/api/script.js"}" data-site-id="${fields.siteId}"></script>`;
    case "plausible":
      return `<script defer data-domain="${fields.domain}" src="https://plausible.io/js/script.js"></script>`;
    case "custom":
      return fields.custom.trim();
    default:
      return "";
  }
}

export function Analytics({ id }: { id: string }) {
  const [provider, setProvider] = useState<AnalyticsProvider>("none");
  const [siteId, setSiteId] = useState("");
  const [src, setSrc] = useState("");
  const [domain, setDomain] = useState("");
  const [custom, setCustom] = useState("");
  const [gsc, setGsc] = useState("");
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState("");
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    api
      .project(id)
      .then((p) => {
        const a = (p.config?.analytics ?? {}) as Record<string, string>;
        setProvider((a.provider as AnalyticsProvider) ?? "none");
        setSiteId(a.site_id ?? "");
        setSrc(a.src ?? "");
        setDomain(a.domain ?? "");
        setCustom(a.custom ?? "");
        setGsc(a.gsc ?? "");
      })
      .catch(() => {})
      .finally(() => setLoaded(true));
  }, [id]);

  const snippet = buildSnippet(provider, { siteId, src, domain, custom });
  const meta = gsc.trim()
    ? `<meta name="google-site-verification" content="${gsc.trim()}">`
    : "";
  const needsFields =
    ((provider === "umami" || provider === "rybbit") && !siteId) ||
    (provider === "plausible" && !domain) ||
    (provider === "custom" && !custom.trim());

  async function save() {
    setError("");
    setSaved(false);
    try {
      await api.patchProject(id, {
        config: {
          analytics: { provider, site_id: siteId, src, domain, custom, gsc },
          analytics_snippet: provider === "none" ? null : snippet,
          analytics_meta: meta || null,
        },
      });
      setSaved(true);
      setTimeout(() => setSaved(false), 2000);
    } catch (e) {
      setError(e instanceof Error ? e.message : "save failed");
    }
  }

  if (!loaded) return <Skeleton className="h-40" />;

  return (
    <div className="max-w-2xl space-y-4">
      <WebAnalytics id={id} />
      <Card className="p-5">
        <h2 className="mb-1 text-sm font-medium">Third-party snippet</h2>
        <p className="mb-4 text-xs text-muted-foreground">
          Injects a tracking snippet into every HTML page on the next static deploy.
          Works with Umami, Rybbit, Plausible, or any provider's script tag.
        </p>
      <div className="grid grid-cols-1 gap-3">
        <ComboBox
          value={provider}
          onChange={(v) => setProvider(v as AnalyticsProvider)}
          options={[
            { value: "none", label: "Disabled" },
            { value: "umami", label: "Umami" },
            { value: "rybbit", label: "Rybbit" },
            { value: "plausible", label: "Plausible" },
            { value: "custom", label: "Custom snippet" },
          ]}
          placeholder="Analytics…"
        />
        {(provider === "umami" || provider === "rybbit") && (
          <>
            <Input
              placeholder={provider === "umami" ? "Website ID" : "Site ID"}
              value={siteId}
              onChange={(e) => setSiteId(e.target.value)}
            />
            <Input
              placeholder={
                provider === "umami"
                  ? "Script URL (default: cloud.umami.is)"
                  : "Script URL (default: app.rybbit.io)"
              }
              value={src}
              onChange={(e) => setSrc(e.target.value)}
            />
          </>
        )}
        {provider === "plausible" && (
          <Input
            placeholder="Domain (e.g. example.com)"
            value={domain}
            onChange={(e) => setDomain(e.target.value)}
          />
        )}
        {provider === "custom" && (
          <textarea
            value={custom}
            onChange={(e) => setCustom(e.target.value)}
            placeholder="<script defer src=…></script>"
            rows={3}
            className="w-full rounded-md border border-border bg-transparent px-3 py-2 font-mono text-sm placeholder:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-ring"
          />
        )}
        <Input
          placeholder="Google Search Console verification token (optional)"
          value={gsc}
          onChange={(e) => setGsc(e.target.value)}
        />
        {((provider !== "none" && !needsFields) || meta) && (
          <pre className="overflow-x-auto rounded-md border border-border bg-black/40 p-3 font-mono text-xs text-muted-foreground">
            {[meta, snippet].filter(Boolean).join("\n")}
          </pre>
        )}
        <Err msg={error} />
        <div className="flex items-center gap-3">
          <Button size="sm" onClick={save} disabled={provider !== "none" && needsFields}>
            Save
          </Button>
          {saved && <span className="text-xs text-muted-foreground">Saved — applies to the next deploy.</span>}
        </div>
      </div>
      </Card>
    </div>
  );
}

export function WebAnalytics({ id }: { id: string }) {
  const [enabled, setEnabled] = useState(false);
  const [data, setData] = useState<Awaited<ReturnType<typeof api.webAnalytics>> | null>(null);
  const [error, setError] = useState("");

  useEffect(() => {
    api
      .project(id)
      .then((p) => {
        const a = (p.config?.web_analytics ?? {}) as { enabled?: boolean };
        setEnabled(!!a.enabled);
      })
      .catch(() => {});
    api
      .webAnalytics(id)
      .then(setData)
      .catch((e) => setError(e instanceof Error ? e.message : "failed to load"));
  }, [id]);

  async function toggle() {
    const next = !enabled;
    setEnabled(next);
    setError("");
    try {
      await api.patchProject(id, { config: { web_analytics: { enabled: next } } });
    } catch (e) {
      setEnabled(!next);
      setError(e instanceof Error ? e.message : "save failed");
    }
  }

  const toggleCls = (on: boolean) =>
    `relative h-5 w-9 shrink-0 rounded-full transition-colors ${on ? "bg-emerald-500" : "bg-muted"}`;
  const knob = (on: boolean) =>
    `absolute top-0.5 h-4 w-4 rounded-full bg-white transition-all ${on ? "left-[18px]" : "left-0.5"}`;

  return (
    <Card className="p-5">
      <div className="flex items-start justify-between gap-4">
        <div>
          <h2 className="mb-1 text-sm font-medium">Web analytics</h2>
          <p className="text-xs text-muted-foreground">
            Built-in, privacy-preserving analytics — pageviews, visitors, referrers, and
            custom events via <code className="font-mono">window.rw.event()</code>. Visitors
            are counted with a daily-rotating hash; no IPs or cookies are stored. Active on
            the next deploy.
          </p>
        </div>
        <button
          onClick={toggle}
          role="switch"
          aria-checked={enabled}
          aria-label="Enable analytics"
          className={toggleCls(enabled)}
        >
          <span className={knob(enabled)} />
        </button>
      </div>
      {data && data.views > 0 && (
        <div className="mt-4 grid grid-cols-1 gap-4">
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <div className="rounded-md border border-border p-3">
              <div className="text-2xl font-semibold">{data.views.toLocaleString()}</div>
              <div className="text-xs text-muted-foreground">Pageviews · {data.days}d</div>
            </div>
            <div className="rounded-md border border-border p-3">
              <div className="text-2xl font-semibold">{data.visitors.toLocaleString()}</div>
              <div className="text-xs text-muted-foreground">Visitors · {data.days}d</div>
            </div>
          </div>
          {data.pages.length > 0 && (
            <div>
              <h3 className="mb-2 text-xs font-medium text-muted-foreground">Top pages</h3>
              <table className="w-full text-sm">
                <tbody>
                  {data.pages.map((p) => (
                    <tr key={p.path} className="border-b border-border/50 last:border-0">
                      <td className="py-1 font-mono text-xs">{p.path}</td>
                      <td className="py-1 text-right text-xs">{p.views}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
          {data.referrers.length > 0 && (
            <div>
              <h3 className="mb-2 text-xs font-medium text-muted-foreground">Referrers</h3>
              <table className="w-full text-sm">
                <tbody>
                  {data.referrers.map((r) => (
                    <tr key={r.host} className="border-b border-border/50 last:border-0">
                      <td className="py-1 font-mono text-xs">{r.host}</td>
                      <td className="py-1 text-right text-xs">{r.views}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
          {data.events.length > 0 && (
            <div>
              <h3 className="mb-2 text-xs font-medium text-muted-foreground">Custom events</h3>
              <table className="w-full text-sm">
                <tbody>
                  {data.events.map((e) => (
                    <tr key={e.name} className="border-b border-border/50 last:border-0">
                      <td className="py-1 font-mono text-xs">{e.name}</td>
                      <td className="py-1 text-right text-xs">{e.count}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </div>
      )}
      {data && data.views === 0 && enabled && (
        <p className="mt-3 text-xs text-muted-foreground">
          No pageviews yet — the beacon activates on the next deploy.
        </p>
      )}
      <Err msg={error} />
    </Card>
  );
}

// p75 thresholds → green / amber / red (Core Web Vitals ratings).
