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
import { AvatarRow, Badge, Button, Card, Input, Skeleton, StatusDot, displayStatus } from "@/components/ui";
import { filesToTarGz } from "@/lib/tarball";
import { duration, firstLine, timeAgo } from "@/lib/utils";
import { useEffect, useRef, useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";

const TABS = [
  "deployments",
  "logs",
  "environment",
  "analytics",
  "speed",
  "cron",
  "redirects",
  "domains",
  "webhooks",
  "settings",
] as const;
type Tab = (typeof TABS)[number];

type AnalyticsProvider = "none" | "umami" | "rybbit" | "plausible" | "custom";

function buildSnippet(p: AnalyticsProvider, fields: { siteId: string; src: string; domain: string; custom: string }): string {
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

function Analytics({ id }: { id: string }) {
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
      <div className="grid gap-3">
        <select
          className={selectCls}
          value={provider}
          onChange={(e) => setProvider(e.target.value as AnalyticsProvider)}
        >
          <option value="none">Disabled</option>
          <option value="umami">Umami</option>
          <option value="rybbit">Rybbit</option>
          <option value="plausible">Plausible</option>
          <option value="custom">Custom snippet</option>
        </select>
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

function WebAnalytics({ id }: { id: string }) {
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
        <button onClick={toggle} className={toggleCls(enabled)}>
          <span className={knob(enabled)} />
        </button>
      </div>
      {data && data.views > 0 && (
        <div className="mt-4 grid gap-4">
          <div className="grid grid-cols-2 gap-3">
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
const VITALS: {
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

function Speed({ id }: { id: string }) {
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

const selectCls =
  "h-9 w-full rounded-md border border-border bg-transparent px-3 text-sm focus:outline-none focus:ring-1 focus:ring-ring";
const inputCls =
  "w-full rounded-md border border-border bg-transparent px-3 py-2 font-mono text-xs focus:outline-none focus:ring-1 focus:ring-ring";

function Err({ msg }: { msg: string }) {
  return msg ? <p className="mt-2 text-xs text-destructive">{msg}</p> : null;
}

function Deployments({ id }: { id: string }) {
  const nav = useNavigate();
  const [deployments, setDeployments] = useState<Deployment[] | null>(null);
  const [deploying, setDeploying] = useState(false);
  const [dragging, setDragging] = useState(false);
  const [uploading, setUploading] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    const load = () =>
      api.deployments(id).then((r) => setDeployments(r.deployments)).catch((e) => setError(e.message));
    load();
    const es = projectEventsStream(id);
    es.onmessage = () => load();
    return () => es.close();
  }, [id]);

  async function drop(e: React.DragEvent) {
    e.preventDefault();
    setDragging(false);
    const files = [...(e.dataTransfer?.files ?? [])];
    if (files.length === 0 || uploading) return;
    setError("");
    setUploading(true);
    try {
      const single = files[0];
      const blob =
        files.length === 1 && single.name.endsWith(".tar.gz")
          ? single
          : await filesToTarGz(files);
      const d = await api.uploadDeploy(id, blob);
      nav(`/deployments/${d.id}`);
    } catch (err) {
      setError(err instanceof Error ? err.message : "upload failed");
      setUploading(false);
    }
  }

  return (
    <div
      onDragOver={(e) => {
        if (e.dataTransfer.types.includes("Files")) {
          e.preventDefault();
          setDragging(true);
        }
      }}
      onDragLeave={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node)) setDragging(false);
      }}
      onDrop={drop}
      className="relative"
    >
      {dragging && (
        <div className="pointer-events-none absolute inset-0 z-10 flex items-center justify-center rounded-lg border-2 border-dashed border-primary/60 bg-background/80">
          <div className="text-center">
            <p className="font-medium">Drop to deploy</p>
            <p className="mt-1 text-xs text-muted-foreground">
              HTML files, folders, or a .tar.gz — live in seconds
            </p>
          </div>
        </div>
      )}
      <div className="mb-4 flex items-center justify-between">
        <span className="text-xs text-muted-foreground">
          {uploading ? "Uploading…" : "Tip: drop files here to deploy instantly"}
        </span>
        <Button
          size="sm"
          disabled={deploying}
          onClick={() => {
            setDeploying(true);
            setError("");
            api
              .deploy(id)
              .then((d) => nav(`/deployments/${d.id}`))
              .catch((e) => setError(e.message))
              .finally(() => setDeploying(false));
          }}
        >
          {deploying ? "Deploying…" : "Deploy now"}
        </Button>
      </div>
      <Err msg={error} />
      <div className="stagger grid gap-2">
        {deployments === null && [0, 1, 2].map((i) => <Skeleton key={i} className="h-[64px]" />)}
        {deployments?.map((d) => (
          <Link key={d.id} to={`/deployments/${d.id}`}>
            <Card className="flex items-center justify-between p-4 transition-colors hover:border-muted-foreground/25 hover:bg-accent/50">
              <div className="flex min-w-0 items-center gap-4">
                <StatusDot status={d.status} conclusion={d.conclusion} computed={d.computed_status} />
                <div className="min-w-0">
                  <div className="truncate text-sm">
                    <span className="font-mono">{d.commit_sha.slice(0, 7)}</span>
                    {firstLine(d.commit_meta?.message) && (
                      <span className="ml-2 text-muted-foreground">
                        {firstLine(d.commit_meta?.message)}
                      </span>
                    )}
                  </div>
                  <div className="text-xs text-muted-foreground">
                    {d.branch} · {d.environment_id}
                    {d.commit_meta?.author && ` · ${d.commit_meta.author}`}
                  </div>
                </div>
                <Badge variant={displayStatus(d).variant} className="shrink-0">
                  {displayStatus(d).label}
                </Badge>
              </div>
              <div className="shrink-0 text-right text-xs text-muted-foreground">
                <div>{timeAgo(d.created_at)}</div>
                {duration(d.created_at, d.concluded_at) && (
                  <div className="font-mono">{duration(d.created_at, d.concluded_at)}</div>
                )}
              </div>
            </Card>
          </Link>
        ))}
        {deployments !== null && deployments.length === 0 && (
          <p className="text-sm text-muted-foreground">
            No deployments yet — hit Deploy now, or drop files here.
          </p>
        )}
      </div>
    </div>
  );
}

function Environment({ id }: { id: string }) {
  const [vars, setVars] = useState<EnvVar[]>([]);
  const [key, setKey] = useState("");
  const [value, setValue] = useState("");
  const [bulk, setBulk] = useState(false);
  const [bulkText, setBulkText] = useState("");
  const [error, setError] = useState("");

  const load = () => api.getEnv(id).then((r) => setVars(r.env)).catch((e) => setError(e.message));
  useEffect(() => {
    load();
  }, [id]);

  async function add(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    try {
      await api.patchEnv(id, [{ key, value }]);
      setKey("");
      setValue("");
      load();
    } catch (err) {
      setError(err instanceof Error ? err.message : "failed");
    }
  }

  async function addBulk(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    const pairs = bulkText
      .split("\n")
      .map((l) => l.trim())
      .filter((l) => l && !l.startsWith("#") && l.includes("="))
      .map((l) => {
        const i = l.indexOf("=");
        return { key: l.slice(0, i).trim(), value: l.slice(i + 1).trim() };
      })
      .filter((p) => p.key);
    if (pairs.length === 0) {
      setError("no KEY=value lines found");
      return;
    }
    try {
      await api.patchEnv(id, pairs);
      setBulkText("");
      setBulk(false);
      load();
    } catch (err) {
      setError(err instanceof Error ? err.message : "failed");
    }
  }

  return (
    <>
      {bulk ? (
        <form onSubmit={addBulk} className="mb-4 grid gap-2">
          <textarea
            value={bulkText}
            onChange={(e) => setBulkText(e.target.value)}
            placeholder={"KEY=value\nANOTHER=thing\n# comments ignored"}
            rows={6}
            autoFocus
            className="w-full rounded-md border border-border bg-transparent px-3 py-2 font-mono text-sm placeholder:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-ring"
          />
          <div className="flex gap-2">
            <Button type="submit" size="sm" disabled={!bulkText.trim()}>
              Add all
            </Button>
            <Button type="button" variant="ghost" size="sm" onClick={() => setBulk(false)}>
              Cancel
            </Button>
          </div>
        </form>
      ) : (
        <form onSubmit={add} className="mb-4 grid grid-cols-[1fr_1fr_auto_auto] gap-2">
          <Input placeholder="KEY" value={key} onChange={(e) => setKey(e.target.value)} required />
          <Input placeholder="value" value={value} onChange={(e) => setValue(e.target.value)} />
          <Button type="submit" size="sm">
            Add
          </Button>
          <Button type="button" variant="outline" size="sm" onClick={() => setBulk(true)}>
            Paste .env
          </Button>
        </form>
      )}
      <Err msg={error} />
      <div className="stagger grid gap-2">
        {vars.map((v) => (
          <Card key={`${v.key}:${v.environment}`} className="flex items-center justify-between p-3">
            <div className="font-mono text-sm">
              {v.key}
              {v.environment && (
                <span className="ml-2 text-xs text-muted-foreground">({v.environment})</span>
              )}
            </div>
            <div className="flex items-center gap-3">
              <span className="font-mono text-xs text-muted-foreground">{v.value || "••••"}</span>
              <Button
                variant="ghost"
                size="sm"
                onClick={() =>
                  api.patchEnv(id, [{ key: v.key, delete: true }]).then(load).catch(() => {})
                }
              >
                Remove
              </Button>
            </div>
          </Card>
        ))}
        {vars.length === 0 && (
          <p className="text-sm text-muted-foreground">No environment variables yet.</p>
        )}
      </div>
    </>
  );
}

function Cron({ id }: { id: string }) {
  const [jobs, setJobs] = useState<CronJob[]>([]);
  const [name, setName] = useState("");
  const [schedule, setSchedule] = useState("");
  const [branch, setBranch] = useState("main");
  const [error, setError] = useState("");

  const load = () => api.cron(id).then((r) => setJobs(r.cron_jobs)).catch((e) => setError(e.message));
  useEffect(() => {
    load();
  }, [id]);

  async function add(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    try {
      await api.createCron(id, { name, schedule, branch });
      setName("");
      setSchedule("");
      load();
    } catch (err) {
      setError(err instanceof Error ? err.message : "failed");
    }
  }

  return (
    <>
      <form onSubmit={add} className="mb-4 grid grid-cols-[1fr_1fr_120px_auto] gap-2">
        <Input placeholder="Name" value={name} onChange={(e) => setName(e.target.value)} required />
        <Input
          placeholder="every 30 minutes"
          value={schedule}
          onChange={(e) => setSchedule(e.target.value)}
          required
        />
        <Input placeholder="branch" value={branch} onChange={(e) => setBranch(e.target.value)} />
        <Button type="submit" size="sm">
          Add
        </Button>
      </form>
      <Err msg={error} />
      <div className="stagger grid gap-2">
        {jobs.map((j) => (
          <Card key={j.id} className="flex items-center justify-between p-3">
            <div>
              <div className="text-sm font-medium">
                {j.name} <span className="font-mono text-xs text-muted-foreground">{j.schedule}</span>
              </div>
              <div className="text-xs text-muted-foreground">
                {j.branch} · next {j.next_run_at ? new Date(j.next_run_at).toLocaleString() : "—"}
              </div>
            </div>
            <div className="flex items-center gap-2">
              <Badge variant={j.enabled ? "success" : "secondary"}>
                {j.enabled ? "enabled" : "disabled"}
              </Badge>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => api.patchCron(id, j.id, !j.enabled).then(load).catch(() => {})}
              >
                {j.enabled ? "Disable" : "Enable"}
              </Button>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => api.deleteCron(id, j.id).then(load).catch(() => {})}
              >
                Delete
              </Button>
            </div>
          </Card>
        ))}
        {jobs.length === 0 && (
          <p className="text-sm text-muted-foreground">No cron jobs yet.</p>
        )}
      </div>
    </>
  );
}

function Redirects({ id }: { id: string }) {
  const [rules, setRules] = useState<RedirectRule[]>([]);
  const [source, setSource] = useState("");
  const [target, setTarget] = useState("");
  const [code, setCode] = useState("301");
  const [error, setError] = useState("");

  const load = () =>
    api.redirects(id).then((r) => setRules(r.redirect_rules)).catch((e) => setError(e.message));
  useEffect(() => {
    load();
  }, [id]);

  async function add(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    try {
      await api.createRedirect(id, {
        source_path: source,
        target_url: target,
        status_code: Number(code),
      });
      setSource("");
      setTarget("");
      load();
    } catch (err) {
      setError(err instanceof Error ? err.message : "failed");
    }
  }

  return (
    <>
      <form onSubmit={add} className="mb-4 grid grid-cols-[1fr_1fr_90px_auto] gap-2">
        <Input
          placeholder="/old-path"
          value={source}
          onChange={(e) => setSource(e.target.value)}
          required
        />
        <Input
          placeholder="https://example.com/new"
          value={target}
          onChange={(e) => setTarget(e.target.value)}
          required
        />
        <select className={selectCls} value={code} onChange={(e) => setCode(e.target.value)}>
          {["301", "302", "307", "308"].map((c) => (
            <option key={c}>{c}</option>
          ))}
        </select>
        <Button type="submit" size="sm">
          Add
        </Button>
      </form>
      <Err msg={error} />
      <div className="stagger grid gap-2">
        {rules.map((r) => (
          <Card key={r.id} className="flex items-center justify-between p-3">
            <div className="font-mono text-sm">
              {r.source_path} → {r.target_url}
              <span className="ml-2 text-xs text-muted-foreground">{r.status_code}</span>
            </div>
            <div className="flex items-center gap-2">
              <Badge variant={r.enabled ? "success" : "secondary"}>
                {r.enabled ? "on" : "off"}
              </Badge>
              <Button
                variant="ghost"
                size="sm"
                onClick={() =>
                  api.patchRedirect(id, r.id, { enabled: !r.enabled }).then(load).catch(() => {})
                }
              >
                {r.enabled ? "Disable" : "Enable"}
              </Button>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => api.deleteRedirect(id, r.id).then(load).catch(() => {})}
              >
                Delete
              </Button>
            </div>
          </Card>
        ))}
        {rules.length === 0 && (
          <p className="text-sm text-muted-foreground">No redirect rules yet.</p>
        )}
      </div>
    </>
  );
}

function Domains({ id }: { id: string }) {
  const [domains, setDomains] = useState<Domain[]>([]);
  const [hostname, setHostname] = useState("");
  const [error, setError] = useState("");

  const load = () =>
    api.domains(id).then((r) => setDomains(r.domains)).catch((e) => setError(e.message));
  useEffect(() => {
    load();
  }, [id]);

  async function add(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    try {
      await api.addDomain(id, hostname);
      setHostname("");
      load();
    } catch (err) {
      setError(err instanceof Error ? err.message : "failed");
    }
  }

  return (
    <>
      <form onSubmit={add} className="mb-4 flex gap-2">
        <Input
          placeholder="app.example.com"
          value={hostname}
          onChange={(e) => setHostname(e.target.value)}
          required
        />
        <Button type="submit" size="sm">
          Add domain
        </Button>
      </form>
      <Err msg={error} />
      <div className="stagger grid gap-2">
        {domains.map((d) => (
          <Card key={d.id} className="flex items-center justify-between p-3">
            <div className="font-mono text-sm">{d.hostname}</div>
            <div className="flex items-center gap-2">
              <Badge variant={d.status === "active" ? "success" : "warning"}>{d.status}</Badge>
              {d.status !== "active" && (
                <>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() =>
                      api.assignCloudflareDomain(id, d.id).then(load).catch((e) => setError(e.message))
                    }
                  >
                    Assign via Cloudflare
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() =>
                      api.verifyDomain(id, d.id).then(load).catch((e) => setError(e.message))
                    }
                  >
                    Verify
                  </Button>
                </>
              )}
              <Button
                variant="ghost"
                size="sm"
                onClick={() => api.deleteDomain(id, d.id).then(load).catch(() => {})}
              >
                Remove
              </Button>
            </div>
          </Card>
        ))}
        {domains.length === 0 && (
          <p className="text-sm text-muted-foreground">No domains yet.</p>
        )}
      </div>
    </>
  );
}

function Webhooks({ id }: { id: string }) {
  const [hooks, setHooks] = useState<Webhook[]>([]);
  const [name, setName] = useState("");
  const [url, setUrl] = useState("");
  const [secret, setSecret] = useState("");
  const [error, setError] = useState("");

  const load = () =>
    api.webhooks(id).then((r) => setHooks(r.webhooks)).catch((e) => setError(e.message));
  useEffect(() => {
    load();
  }, [id]);

  async function add(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    try {
      await api.createWebhook(id, { name, url, secret: secret || undefined });
      setName("");
      setUrl("");
      setSecret("");
      load();
    } catch (err) {
      setError(err instanceof Error ? err.message : "failed");
    }
  }

  return (
    <>
      <form onSubmit={add} className="mb-4 grid grid-cols-[1fr_2fr_1fr_auto] gap-2">
        <Input placeholder="Name" value={name} onChange={(e) => setName(e.target.value)} required />
        <Input
          placeholder="https://example.com/hook"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          required
        />
        <Input
          placeholder="secret (optional)"
          value={secret}
          onChange={(e) => setSecret(e.target.value)}
        />
        <Button type="submit" size="sm">
          Add
        </Button>
      </form>
      <Err msg={error} />
      <div className="stagger grid gap-2">
        {hooks.map((w) => (
          <Card key={w.id} className="flex items-center justify-between p-3">
            <div>
              <div className="text-sm font-medium">{w.name}</div>
              <div className="font-mono text-xs text-muted-foreground">
                {w.url} {w.has_secret && "· signed"} {w.events.length > 0 && `· ${w.events.join(",")}`}
              </div>
            </div>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => api.deleteWebhook(id, w.id).then(load).catch(() => {})}
            >
              Delete
            </Button>
          </Card>
        ))}
        {hooks.length === 0 && (
          <p className="text-sm text-muted-foreground">No webhooks yet.</p>
        )}
      </div>
    </>
  );
}

function Settings({ id, project }: { id: string; project: Project | null }) {
  const [nodes, setNodes] = useState<RemoteNode[]>([]);
  const [error, setError] = useState("");
  const [msg, setMsg] = useState("");
  const importRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    api.nodes().then((r) => setNodes(r.nodes)).catch(() => {});
  }, []);

  async function assign(nodeId: string) {
    setError("");
    setMsg("");
    try {
      await api.patchProject(id, { remote_node_id: nodeId || null });
      setMsg("Saved.");
    } catch (err) {
      setError(err instanceof Error ? err.message : "failed");
    }
  }

  async function doExport() {
    const data = await api.exportProject(id);
    const blob = new Blob([JSON.stringify(data, null, 2)], { type: "application/json" });
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = `${project?.name ?? "project"}-export.json`;
    a.click();
    URL.revokeObjectURL(a.href);
  }

  async function doImport(e: React.ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0];
    if (!file) return;
    setError("");
    setMsg("");
    try {
      await api.importProject(id, JSON.parse(await file.text()));
      setMsg("Imported. Config merged; env vars and redirect rules added.");
    } catch (err) {
      setError(err instanceof Error ? err.message : "import failed");
    }
    e.target.value = "";
  }

  const [avatarKey, setAvatarKey] = useState(0);
  return (
    <div className="grid gap-4">
      {project && (
        <Card className="p-4">
          <h3 className="mb-2 text-sm font-medium">Avatar</h3>
          <AvatarRow
            key={avatarKey}
            kind="project"
            id={project.id}
            name={project.name}
            hasAvatar={project.has_avatar}
            onChanged={() => {
              project.has_avatar = !project.has_avatar;
              setAvatarKey((k) => k + 1);
            }}
          />
        </Card>
      )}
      <Card className="p-4">
        <h3 className="mb-2 text-sm font-medium">Remote Docker node</h3>
        <p className="mb-3 text-xs text-muted-foreground">
          Deploy to a remote Docker daemon. Requires the node published port to be reachable from
          Traefik.
        </p>
        <select
          className={selectCls}
          value={project?.remote_node_id ?? ""}
          onChange={(e) => assign(e.target.value)}
        >
          <option value="">Local daemon</option>
          {nodes.map((n) => (
            <option key={n.id} value={n.id}>
              {n.name} ({n.host}){n.status !== "online" ? " — offline" : ""}
            </option>
          ))}
        </select>
      </Card>
      <Environments id={id} project={project} />
      <DockerBuild id={id} project={project} />
      <HealthCheck id={id} project={project} />
      <Retention id={id} project={project} />
      <DeployRules id={id} project={project} />
      <StatusPageCard id={id} project={project} />
      <Firewall id={id} project={project} />
      <Card className="p-4">
        <h3 className="mb-2 text-sm font-medium">Export / import</h3>
        <div className="flex gap-2">
          <Button variant="outline" size="sm" onClick={() => doExport().catch(() => {})}>
            Export JSON
          </Button>
          <Button variant="outline" size="sm" onClick={() => importRef.current?.click()}>
            Import JSON
          </Button>
          <input ref={importRef} type="file" accept=".json" hidden onChange={doImport} />
        </div>
      </Card>
      {project && <DangerZone project={project} />}
      <Err msg={error} />
      {msg && <p className="text-xs text-muted-foreground">{msg}</p>}
    </div>
  );
}

function DangerZone({ project }: { project: Project }) {
  const nav = useNavigate();
  const [confirm, setConfirm] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");

  async function destroy() {
    setBusy(true);
    setErr("");
    try {
      await api.deleteProject(project.id, confirm);
      nav("/projects");
    } catch (e) {
      setErr(e instanceof Error ? e.message : "failed");
      setBusy(false);
    }
  }

  return (
    <Card className="border-destructive/40 p-4">
      <h3 className="mb-1 text-sm font-medium text-destructive">Delete project</h3>
      <p className="mb-3 text-xs text-muted-foreground">
        Removes all deployments, containers, domains and edge config. Type{" "}
        <span className="font-mono text-foreground">{project.name}</span> to confirm.
      </p>
      <div className="flex gap-2">
        <Input
          value={confirm}
          onChange={(e) => setConfirm(e.target.value)}
          placeholder={project.name}
          className="max-w-56"
        />
        <Button
          variant="destructive"
          size="sm"
          disabled={busy || confirm !== project.name}
          onClick={destroy}
        >
          Delete
        </Button>
      </div>
      {err && <p className="mt-2 text-xs text-destructive">{err}</p>}
    </Card>
  );
}

type Env = { id: string; name: string; slug: string; branch?: string; status?: string };

function Environments({ id, project }: { id: string; project: Project | null }) {
  const [envs, setEnvs] = useState<Env[]>([]);
  const [name, setName] = useState("");
  const [branch, setBranch] = useState("");
  const [msg, setMsg] = useState("");
  const [error, setError] = useState("");

  useEffect(() => {
    setEnvs((project?.environments as Env[] | undefined) ?? []);
  }, [project]);

  async function save(next: Env[]) {
    setError("");
    setMsg("");
    try {
      await api.patchProject(id, { environments: next });
      setEnvs(next);
      setMsg("Saved — applies to the next deploy.");
    } catch (err) {
      setError(err instanceof Error ? err.message : "failed");
    }
  }

  async function add(e: React.FormEvent) {
    e.preventDefault();
    const slug = name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
    if (!slug) return;
    await save([...envs, { id: `env-${slug}`, name, slug, branch: branch || "*", status: "active" }]);
    setName("");
    setBranch("");
  }

  return (
    <Card className="p-4">
      <h3 className="mb-2 text-sm font-medium">Environments</h3>
      <p className="mb-3 text-xs text-muted-foreground">
        Deploy targets matched by branch pattern (<code>*</code> = any). Each gets its own
        environment domain.
      </p>
      <label className="mb-3 flex items-center gap-2 text-sm">
        <input
          type="checkbox"
          className="accent-primary"
          checked={(project?.config?.preview_environments as boolean | undefined) !== false}
          onChange={async (e) => {
            setError("");
            try {
              await api.patchProject(id, {
                config: { ...(project?.config ?? {}), preview_environments: e.target.checked },
              });
              setMsg(e.target.checked ? "Preview environments enabled." : "Preview environments disabled.");
            } catch (err) {
              setError(err instanceof Error ? err.message : "failed");
            }
          }}
        />
        <span>
          Preview environments
          <span className="block text-xs text-muted-foreground">
            Branches matching no environment deploy as previews under their branch URL.
          </span>
        </span>
      </label>
      <div className="mb-3 grid gap-2">
        {envs.map((env) => (
          <div
            key={env.id}
            className="flex items-center justify-between rounded-md border border-border px-3 py-2"
          >
            <div className="text-sm">
              {env.name}
              <span className="ml-2 font-mono text-xs text-muted-foreground">
                {env.slug} · {env.branch || "*"}
              </span>
            </div>
            <div className="flex items-center gap-2">
              <Badge variant={env.status === "active" ? "success" : "secondary"}>
                {env.status ?? "active"}
              </Badge>
              {env.slug !== "prod" && (
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => save(envs.filter((x) => x.id !== env.id))}
                >
                  Remove
                </Button>
              )}
            </div>
          </div>
        ))}
      </div>
      <form onSubmit={add} className="grid grid-cols-[1fr_1fr_auto] gap-2">
        <Input
          placeholder="staging"
          value={name}
          onChange={(e) => setName(e.target.value)}
          required
        />
        <Input
          placeholder="branch pattern (e.g. develop or *)"
          value={branch}
          onChange={(e) => setBranch(e.target.value)}
        />
        <Button type="submit" size="sm">
          Add
        </Button>
      </form>
      <Err msg={error} />
      {msg && <p className="mt-2 text-xs text-muted-foreground">{msg}</p>}
    </Card>
  );
}

function DockerBuild({ id, project }: { id: string; project: Project | null }) {
  const [df, setDf] = useState("");
  const [args, setArgs] = useState("");
  const [msg, setMsg] = useState("");
  const [error, setError] = useState("");

  useEffect(() => {
    setDf((project?.config?.dockerfile_path as string) ?? "");
    const ba = (project?.config?.build_args ?? {}) as Record<string, string>;
    setArgs(Object.entries(ba).map(([k, v]) => `${k}=${v}`).join("\n"));
  }, [project?.config]);

  async function save() {
    setError("");
    setMsg("");
    const path = df.trim();
    if (path && (path.startsWith("/") || path.split("/").includes(".."))) {
      setError("dockerfile path must be relative to the repo");
      return;
    }
    const buildArgs: Record<string, string> = {};
    for (const line of args.split("\n")) {
      const t = line.trim();
      if (!t || t.startsWith("#")) continue;
      const i = t.indexOf("=");
      if (i < 1) {
        setError(`invalid build arg: ${t}`);
        return;
      }
      buildArgs[t.slice(0, i).trim()] = t.slice(i + 1).trim();
    }
    try {
      await api.patchProject(id, {
        config: {
          dockerfile_path: path || null,
          build_args: buildArgs,
        },
      });
      setMsg(path ? "Saved — next deploy builds the Dockerfile into an image." : "Saved.");
      setTimeout(() => setMsg(""), 2500);
    } catch (e) {
      setError(e instanceof Error ? e.message : "save failed");
    }
  }

  return (
    <Card className="p-4">
      <h3 className="mb-1 text-sm font-medium">Dockerfile</h3>
      <p className="mb-3 text-xs text-muted-foreground">
        Build the repo with <code className="font-mono">docker build</code> instead of a preset —
        the image's own command serves the app. Path is relative to the repo root (or the root
        directory if set).
      </p>
      <div className="grid gap-3">
        <Input
          placeholder="Dockerfile"
          value={df}
          onChange={(e) => setDf(e.target.value)}
          className="font-mono"
        />
        <textarea
          className={inputCls}
          rows={3}
          placeholder={"Build args (one per line)\nNODE_ENV=production"}
          value={args}
          onChange={(e) => setArgs(e.target.value)}
        />
        <div>
          <Button size="sm" onClick={save}>
            Save
          </Button>
        </div>
      </div>
      <Err msg={error} />
      {msg && <p className="mt-2 text-xs text-muted-foreground">{msg}</p>}
    </Card>
  );
}

function HealthCheck({ id, project }: { id: string; project: Project | null }) {
  const [path, setPath] = useState("");
  const [msg, setMsg] = useState("");
  const [error, setError] = useState("");

  useEffect(() => {
    const hc = project?.config?.health_check;
    setPath(typeof hc === "string" ? hc : ((hc as { path?: string })?.path ?? ""));
  }, [project?.config]);

  async function save() {
    setError("");
    setMsg("");
    const p = path.trim();
    if (p && !p.startsWith("/")) {
      setError("health check path must start with /");
      return;
    }
    try {
      await api.patchProject(id, {
        config: { health_check: p || null },
      });
      setMsg(p ? "Saved — applies to the next deploy." : "Saved — health checks disabled.");
      setTimeout(() => setMsg(""), 2500);
    } catch (e) {
      setError(e instanceof Error ? e.message : "save failed");
    }
  }

  return (
    <Card className="p-4">
      <h3 className="mb-1 text-sm font-medium">Health check</h3>
      <p className="mb-3 text-xs text-muted-foreground">
        HTTP path probed on running containers (e.g. <code className="font-mono">/healthz</code>).
        Repeated failures mark the deployment <em>unhealthy</em> and notify the team — catches
        "container up, app dead". Applies to deployments created after saving.
      </p>
      <div className="flex gap-2">
        <Input
          placeholder="/healthz"
          value={path}
          onChange={(e) => setPath(e.target.value)}
          className="font-mono"
        />
        <Button size="sm" onClick={save}>
          Save
        </Button>
      </div>
      <Err msg={error} />
      {msg && <p className="mt-2 text-xs text-muted-foreground">{msg}</p>}
    </Card>
  );
}

interface DeployRulesCfg {
  auto_deploy?: boolean;
  deploy_branches?: string;
  ignored_authors?: string;
  skip_merge_commits?: boolean;
  preview_comment?: boolean;
  paths?: string;
}

function DeployRules({ id, project }: { id: string; project: Project | null }) {
  const [rules, setRules] = useState<DeployRulesCfg>({});
  const [msg, setMsg] = useState("");
  const [error, setError] = useState("");

  useEffect(() => {
    setRules((project?.config?.deployment_rules as DeployRulesCfg) ?? {});
  }, [project?.config]);

  async function save() {
    setError("");
    setMsg("");
    try {
      await api.patchProject(id, { config: { deployment_rules: rules } });
      setMsg("Saved — applies to the next push.");
      setTimeout(() => setMsg(""), 2500);
    } catch (e) {
      setError(e instanceof Error ? e.message : "save failed");
    }
  }

  const set = (patch: Partial<DeployRulesCfg>) => setRules((r) => ({ ...r, ...patch }));

  return (
    <Card className="p-4">
      <h3 className="mb-1 text-sm font-medium">Deploy rules</h3>
      <p className="mb-3 text-xs text-muted-foreground">
        Filter which pushes deploy. <code className="font-mono">paths</code> limits deploys to
        pushes touching matching files — glob list like{" "}
        <code className="font-mono">apps/web/**,packages/shared/**</code> (monorepo filtering).
      </p>
      <div className="grid gap-3">
        <label className="flex items-center gap-2 text-xs">
          <input
            type="checkbox"
            checked={rules.auto_deploy !== false}
            onChange={(e) => set({ auto_deploy: e.target.checked })}
          />
          Auto-deploy on push
        </label>
        <div className="grid grid-cols-2 gap-2">
          <div>
            <label className="mb-1 block text-xs text-muted-foreground">Branches</label>
            <Input
              placeholder="main,master or *"
              value={rules.deploy_branches ?? ""}
              onChange={(e) => set({ deploy_branches: e.target.value })}
              className="font-mono"
            />
          </div>
          <div>
            <label className="mb-1 block text-xs text-muted-foreground">Ignored authors</label>
            <Input
              placeholder="bot,dependabot"
              value={rules.ignored_authors ?? ""}
              onChange={(e) => set({ ignored_authors: e.target.value })}
              className="font-mono"
            />
          </div>
        </div>
        <div>
          <label className="mb-1 block text-xs text-muted-foreground">Deploy only paths</label>
          <Input
            placeholder="apps/web/** — empty deploys on any change"
            value={rules.paths ?? ""}
            onChange={(e) => set({ paths: e.target.value })}
            className="font-mono"
          />
        </div>
        <label className="flex items-center gap-2 text-xs">
          <input
            type="checkbox"
            checked={rules.skip_merge_commits === true}
            onChange={(e) => set({ skip_merge_commits: e.target.checked })}
          />
          Skip merge commits
        </label>
        <label className="flex items-center gap-2 text-xs">
          <input
            type="checkbox"
            checked={rules.preview_comment !== false}
            onChange={(e) => set({ preview_comment: e.target.checked })}
          />
          Comment on pull requests with preview URL
        </label>
        <div>
          <Button size="sm" onClick={save}>
            Save rules
          </Button>
        </div>
      </div>
      <Err msg={error} />
      {msg && <p className="mt-2 text-xs text-muted-foreground">{msg}</p>}
    </Card>
  );
}

function Retention({ id, project }: { id: string; project: Project | null }) {
  const [count, setCount] = useState("");
  const [msg, setMsg] = useState("");
  const [error, setError] = useState("");

  useEffect(() => {
    const n = project?.config?.deployment_retention;
    setCount(typeof n === "number" && n > 0 ? String(n) : "");
  }, [project?.config]);

  async function save() {
    setError("");
    setMsg("");
    const n = count.trim() ? parseInt(count, 10) : 0;
    if (!Number.isInteger(n) || n < 0) {
      setError("enter a positive number or leave empty");
      return;
    }
    try {
      await api.patchProject(id, {
        config: { deployment_retention: n > 0 ? n : null },
      });
      setMsg(
        n > 0
          ? `Saved — keeps the newest ${n} per environment, rollback targets spared.`
          : "Saved — retention disabled.",
      );
      setTimeout(() => setMsg(""), 2500);
    } catch (e) {
      setError(e instanceof Error ? e.message : "save failed");
    }
  }

  return (
    <Card className="p-4">
      <h3 className="mb-1 text-sm font-medium">Deployment retention</h3>
      <p className="mb-3 text-xs text-muted-foreground">
        Keep the newest <em>N</em> completed deployments per environment — older
        rows, metrics, logs, and build artifacts are pruned after each deploy.
        Deployments an alias still points at (including rollback targets) are
        never pruned. Empty keeps everything.
      </p>
      <div className="flex gap-2">
        <Input
          placeholder="e.g. 5 — empty keeps all"
          value={count}
          onChange={(e) => setCount(e.target.value.replace(/\D/g, ""))}
          className="w-44"
        />
        <Button size="sm" onClick={save}>
          Save
        </Button>
      </div>
      <Err msg={error} />
      {msg && <p className="mt-2 text-xs text-muted-foreground">{msg}</p>}
    </Card>
  );
}

function StatusPageCard({ id, project }: { id: string; project: Project | null }) {
  const [enabled, setEnabled] = useState(false);
  const [slug, setSlug] = useState("");
  const [error, setError] = useState("");

  useEffect(() => {
    const sp = (project?.config?.status_page ?? {}) as { enabled?: boolean; slug?: string };
    setEnabled(!!sp.enabled);
    setSlug(sp.slug ?? "");
  }, [project?.config]);

  async function save(next: boolean, s: string) {
    setError("");
    const clean = s.trim().toLowerCase().replace(/[^a-z0-9-]/g, "");
    try {
      await api.patchProject(id, {
        config: { status_page: { enabled: next, slug: clean || undefined } },
      });
      setEnabled(next);
      setSlug(clean);
    } catch (e) {
      setError(e instanceof Error ? e.message : "save failed");
    }
  }

  const url = `${window.location.origin}/status/${slug || id}`;

  return (
    <Card className="p-4">
      <div className="mb-1 flex items-center justify-between">
        <h3 className="text-sm font-medium">Public status page</h3>
        {enabled && <Badge variant="success">live</Badge>}
      </div>
      <p className="mb-3 text-xs text-muted-foreground">
        Publishes each environment's live status and 30-day uptime on an
        unauthenticated page — safe to share outside the team. Nothing beyond
        name, status, and uptime is exposed.
      </p>
      <div className="flex items-center gap-2">
        <Input
          placeholder={`custom slug (default: ${id})`}
          value={slug}
          onChange={(e) => setSlug(e.target.value)}
          className="w-56 font-mono"
        />
        {enabled ? (
          <Button variant="outline" size="sm" onClick={() => save(false, slug)}>
            Disable
          </Button>
        ) : (
          <Button size="sm" onClick={() => save(true, slug)}>
            Enable
          </Button>
        )}
      </div>
      {enabled && (
        <p className="mt-3 text-xs">
          Live at{" "}
          <a href={url} target="_blank" rel="noreferrer" className="font-mono text-primary underline">
            {url}
          </a>
          {slug !== "" && (
            <Button variant="outline" size="sm" className="ml-2" onClick={() => save(true, slug)}>
              Save slug
            </Button>
          )}
        </p>
      )}
      <Err msg={error} />
    </Card>
  );
}

function Firewall({ id, project }: { id: string; project: Project | null }) {
  const [allow, setAllow] = useState("");
  const [avg, setAvg] = useState("");
  const [burst, setBurst] = useState("");
  const [msg, setMsg] = useState("");
  const [error, setError] = useState("");

  useEffect(() => {
    const fw = (project?.config?.firewall ?? {}) as {
      ip_allowlist?: string[];
      rate_limit?: { average?: number; burst?: number };
    };
    setAllow((fw.ip_allowlist ?? []).join("\n"));
    setAvg(fw.rate_limit?.average ? String(fw.rate_limit.average) : "");
    setBurst(fw.rate_limit?.burst ? String(fw.rate_limit.burst) : "");
  }, [project?.config]);

  async function save() {
    setError("");
    setMsg("");
    const cidrs = allow.split(/[\s,]+/).map((s) => s.trim()).filter(Boolean);
    for (const c of cidrs) {
      if (!/^[\d.:a-fA-F]+\/\d+$/.test(c) && !/^\d+\.\d+\.\d+\.\d+$/.test(c)) {
        setError(`not an IP or CIDR: ${c}`);
        return;
      }
    }
    try {
      await api.patchProject(id, {
        config: {
          firewall: {
            ip_allowlist: cidrs,
            rate_limit: avg ? { average: +avg, burst: burst ? +burst : +avg * 2 } : null,
          },
        },
      });
      setMsg("Saved — applies at the edge immediately.");
      setTimeout(() => setMsg(""), 2500);
    } catch (e) {
      setError(e instanceof Error ? e.message : "save failed");
    }
  }

  return (
    <Card className="p-4">
      <h3 className="mb-2 text-sm font-medium">Firewall</h3>
      <p className="mb-3 text-xs text-muted-foreground">
        Applied at the edge (Traefik) — blocked traffic never reaches the app. One IP/CIDR per
        line; leave empty to allow everyone.
      </p>
      <div className="grid gap-3">
        <textarea
          value={allow}
          onChange={(e) => setAllow(e.target.value)}
          placeholder={"IP allowlist — e.g.\n203.0.113.0/24\n198.51.100.7"}
          rows={3}
          className="w-full rounded-md border border-border bg-transparent px-3 py-2 font-mono text-sm placeholder:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-ring"
        />
        <div className="flex items-center gap-2">
          <Input
            className="w-36"
            placeholder="Rate limit /s"
            value={avg}
            onChange={(e) => setAvg(e.target.value.replace(/\D/g, ""))}
          />
          <Input
            className="w-36"
            placeholder="Burst"
            value={burst}
            onChange={(e) => setBurst(e.target.value.replace(/\D/g, ""))}
          />
          <span className="text-xs text-muted-foreground">requests/sec per client IP</span>
        </div>
        <Protection id={id} project={project} />
        <div className="flex items-center gap-3">
          <Button size="sm" onClick={save}>Save</Button>
          {msg && <span className="text-xs text-muted-foreground">{msg}</span>}
        </div>
        <Err msg={error} />
      </div>
    </Card>
  );
}

function Protection({ id, project }: { id: string; project: Project | null }) {
  const [pw, setPw] = useState("");
  const [enabled, setEnabled] = useState(false);
  const [error, setError] = useState("");
  const [msg, setMsg] = useState("");

  useEffect(() => {
    const users = (project?.config?.protection as { users?: string[] } | undefined)?.users;
    setEnabled(!!users?.length);
  }, [project?.config]);

  async function apply(enable: boolean) {
    setError("");
    setMsg("");
    if (enable && !pw) {
      setError("set a password first");
      return;
    }
    try {
      await api.patchProject(id, {
        config: { protection_password: enable ? pw : "" },
      });
      setEnabled(enable);
      setPw("");
      setMsg(enable ? "Protected — preview URLs now ask for a password." : "Protection removed.");
      setTimeout(() => setMsg(""), 2500);
    } catch (e) {
      setError(e instanceof Error ? e.message : "failed");
    }
  }

  return (
    <div className="grid gap-2 border-t border-border pt-3">
      <div className="flex items-center justify-between">
        <span className="text-sm">Deployment protection</span>
        {enabled && <Badge variant="success">on</Badge>}
      </div>
      <p className="text-xs text-muted-foreground">
        HTTP password on every URL except production — previews, branch, and
        environment-id deployments ask for <code className="font-mono">runway</code> + this
        password at the edge.
      </p>
      {enabled ? (
        <div>
          <Button variant="outline" size="sm" onClick={() => apply(false)}>
            Disable protection
          </Button>
        </div>
      ) : (
        <div className="flex gap-2">
          <Input
            type="password"
            placeholder="preview password"
            value={pw}
            onChange={(e) => setPw(e.target.value)}
          />
          <Button size="sm" onClick={() => apply(true)}>
            Protect
          </Button>
        </div>
      )}
      <Err msg={error} />
      {msg && <span className="text-xs text-muted-foreground">{msg}</span>}
    </div>
  );
}

function ProjectLogs({ id }: { id: string }) {
  const [lines, setLines] = useState<string[]>([]);
  const [filter, setFilter] = useState("");
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const pull = () => api.projectLogs(id).then((t) => setLines(t.split("\n").filter(Boolean))).catch(() => {});
    pull();
    const t = setInterval(pull, 5000);
    return () => clearInterval(t);
  }, [id]);

  useEffect(() => {
    ref.current?.scrollTo(0, ref.current.scrollHeight);
  }, [lines.length]);

  const shown = filter
    ? lines.filter((l) => l.toLowerCase().includes(filter.toLowerCase()))
    : lines;

  return (
    <div className="overflow-hidden rounded-lg border border-border bg-black/40">
      <div className="flex items-center justify-between border-b border-border px-3 py-2">
        <span className="text-xs text-muted-foreground">
          merged build + runtime log across recent deployments
        </span>
        <input
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="filter…"
          className="w-40 bg-transparent text-xs text-muted-foreground placeholder:text-muted-foreground/50 focus:text-foreground focus:outline-none"
        />
      </div>
      <div ref={ref} className="max-h-[60vh] overflow-auto p-4 font-mono text-xs leading-relaxed text-zinc-300">
        {shown.length === 0 ? (
          <span className="text-muted-foreground">no logs yet</span>
        ) : (
          shown.map((l, i) => {
            const [ts, dep, ...rest] = l.split("\t");
            return (
              <div key={i} className="flex gap-3">
                <span className="shrink-0 text-muted-foreground/50">
                  {ts?.slice(11, 19)}
                </span>
                <span className="shrink-0 text-muted-foreground">{dep}</span>
                <span className="whitespace-pre-wrap break-all">{rest.join("\t")}</span>
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}

export default function ProjectPage() {
  const { id = "" } = useParams();
  const [project, setProject] = useState<Project | null>(null);
  const [tab, setTab] = useState<Tab>("deployments");
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    api.project(id).then(setProject).catch(() => {});
  }, [id]);

  function copyUrl() {
    if (!project?.url) return;
    navigator.clipboard
      .writeText(project.url)
      .then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      })
      .catch(() => {});
  }

  return (
    <div className="page-enter mx-auto max-w-5xl p-4 sm:p-8">
      <div className="mb-6 flex items-center justify-between">
        <div className="min-w-0">
          <h1 className="text-xl font-semibold">{project?.name ?? "Project"}</h1>
          {project && (
            <div className="mt-1.5 flex items-center gap-3 text-xs text-muted-foreground">
              <span className="font-mono">
                {project.repo_provider}:{project.repo_full_name}
              </span>
              <span className="font-mono">{project.repo_branch}</span>
              {project.url && (
                <span className="inline-flex items-center overflow-hidden rounded-md border border-border">
                  <a
                    href={project.url}
                    target="_blank"
                    className="max-w-40 truncate px-2 py-1 font-mono transition-colors hover:bg-accent hover:text-foreground sm:max-w-none"
                  >
                    {project.url.replace(/^https?:\/\//, "")}
                  </a>
                  <button
                    onClick={copyUrl}
                    className="border-l border-border px-2 py-1 transition-colors hover:bg-accent hover:text-foreground"
                    title="Copy URL"
                  >
                    {copied ? "✓" : "⧉"}
                  </button>
                </span>
              )}
            </div>
          )}
        </div>
      </div>
      <div className="-mx-4 mb-6 flex gap-4 overflow-x-auto border-b border-border px-4 sm:mx-0 sm:px-0">
        {TABS.map((t) => (
          <button
            key={t}
            onClick={() => setTab(t)}
            className={`-mb-px shrink-0 border-b-2 pb-2 text-sm capitalize ${
              tab === t
                ? "border-foreground text-foreground"
                : "border-transparent text-muted-foreground hover:text-foreground"
            }`}
          >
            {t}
          </button>
        ))}
      </div>
      {tab === "deployments" && <Deployments id={id} />}
      {tab === "logs" && <ProjectLogs id={id} />}
      {tab === "environment" && <Environment id={id} />}
      {tab === "analytics" && <Analytics id={id} />}
      {tab === "speed" && <Speed id={id} />}
      {tab === "cron" && <Cron id={id} />}
      {tab === "redirects" && <Redirects id={id} />}
      {tab === "domains" && <Domains id={id} />}
      {tab === "webhooks" && <Webhooks id={id} />}
      {tab === "settings" && <Settings id={id} project={project} />}
    </div>
  );
}
