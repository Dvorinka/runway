import { api, deploymentLogsStream, type Deployment } from "@/lib/api";
import { Badge, Button, Menu, StatusDot, displayStatus, isRunning, type MenuItem } from "@/components/ui";
import { Ansi } from "@/lib/ansi";
import { cn, duration, elapsed, firstLine, timeAgo } from "@/lib/utils";
import { ArrowUpToLine, Copy, FolderTree, Globe, History, ListPlus, RefreshCw, Repeat } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useParams } from "react-router-dom";

const STEPS = ["prepare", "deploy", "finalize", "live"];

// Log lines arrive as "<rfc3339>\t<stream>\t<text>"; render the text.
const logText = (l: string) => {
  const i = l.indexOf("\t");
  const j = i < 0 ? -1 : l.indexOf("\t", i + 1);
  return j < 0 ? l : l.slice(j + 1);
};

// `step`-stream lines are phase markers (prepare/deploy/finalize — the
// lifecycle statuses); they group the stream into collapsible sections
// instead of rendering as text.
const logStream = (l: string) => {
  const i = l.indexOf("\t");
  const j = i < 0 ? -1 : l.indexOf("\t", i + 1);
  return j < 0 ? "" : l.slice(i + 1, j);
};

interface LogSection {
  step: string | null;
  lines: string[];
}

function groupLogLines(lines: string[]): LogSection[] {
  const sections: LogSection[] = [{ step: null, lines: [] }];
  for (const l of lines) {
    if (logStream(l) === "step") {
      sections.push({ step: logText(l), lines: [] });
    } else {
      sections[sections.length - 1].lines.push(l);
    }
  }
  return sections.filter((s) => s.step !== null || s.lines.length > 0);
}

const stepTitle = (step: string) => step.charAt(0).toUpperCase() + step.slice(1);

function fmtBytes(n: number): string {
  if (n >= 1 << 30) return `${(n / (1 << 30)).toFixed(1)} GB`;
  if (n >= 1 << 20) return `${(n / (1 << 20)).toFixed(0)} MB`;
  if (n >= 1 << 10) return `${(n / (1 << 10)).toFixed(0)} KB`;
  return `${n} B`;
}

function Sparkline({ values, max }: { values: number[]; max?: number }) {
  if (values.length < 2) return null;
  const peak = max ?? Math.max(...values, 1e-9);
  const pts = values
    .map((v, i) => `${(i / (values.length - 1)) * 100},${28 - (Math.min(v, peak) / peak) * 26}`)
    .join(" ");
  return (
    <svg viewBox="0 0 100 30" preserveAspectRatio="none" className="mt-1 h-6 w-full">
      <polyline
        points={pts}
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
        className="text-emerald-500/70"
        vectorEffect="non-scaling-stroke"
      />
    </svg>
  );
}

function LiveStats({ id }: { id: string }) {
  const [s, setS] = useState<Awaited<ReturnType<typeof api.deploymentStats>> | null>(null);
  const [hist, setHist] = useState<Awaited<ReturnType<typeof api.deploymentMetrics>>["samples"]>([]);
  useEffect(() => {
    const pull = () => api.deploymentStats(id).then(setS).catch(() => {});
    const pullHist = () => api.deploymentMetrics(id).then((m) => setHist(m.samples)).catch(() => {});
    pull();
    pullHist();
    const t = setInterval(pull, 5000);
    const th = setInterval(pullHist, 30000);
    return () => {
      clearInterval(t);
      clearInterval(th);
    };
  }, [id]);
  if (!s?.running) return null;
  const memPct = s.mem_limit ? Math.min(100, ((s.mem_used ?? 0) / s.mem_limit) * 100) : 0;
  const cards: [string, string, number[]][] = [
    ["cpu", `${s.cpu_pct?.toFixed(1)}%`, hist.map((m) => m.cpu_pct)],
    [
      "memory",
      `${fmtBytes(s.mem_used ?? 0)}${s.mem_limit ? ` / ${fmtBytes(s.mem_limit)}` : ""} (${memPct.toFixed(0)}%)`,
      hist.map((m) => m.mem_used),
    ],
    ["network", `↓${fmtBytes(s.net_rx ?? 0)} ↑${fmtBytes(s.net_tx ?? 0)}`, []],
    ["pids", String(s.pids ?? 0), hist.map((m) => m.pids)],
  ];
  return (
    <div className="mb-4 grid grid-cols-2 gap-2 sm:grid-cols-4">
      {cards.map(([k, v, series]) => (
        <div key={k} className="rounded-md border border-border px-3 py-2">
          <div className="text-[10px] uppercase tracking-wide text-muted-foreground">{k}</div>
          <div className="font-mono text-sm tabular-nums">{v}</div>
          <Sparkline values={series} max={k === "memory" ? s.mem_limit : undefined} />
        </div>
      ))}
    </div>
  );
}

function DeployInfo({ dep, active }: { dep: Deployment; active: boolean }) {
  const [copied, setCopied] = useState(false);
  const envUrl = dep.urls?.environment;
  const immUrl = dep.urls?.immutable;
  const mainUrl = envUrl ?? immUrl ?? null;
  const extraUrls = [immUrl && immUrl !== envUrl ? immUrl : null, dep.urls?.branch]
    .filter(Boolean) as string[];

  function copy(text: string) {
    navigator.clipboard
      .writeText(text.startsWith("http") ? text : `https://${text}`)
      .then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      })
      .catch(() => {});
  }

  const rows: [string, React.ReactNode][] = [
    [
      "Status",
      <span key="s" className="flex items-center gap-2">
        <Badge variant={displayStatus(dep).variant}>{displayStatus(dep).label}</Badge>
        <span className="text-xs text-muted-foreground">
          {timeAgo(dep.created_at)}
          {active
            ? ` · running ${elapsed(dep.created_at)}`
            : duration(dep.created_at, dep.concluded_at)
              ? ` · took ${duration(dep.created_at, dep.concluded_at)}`
              : ""}
          {dep.trigger ? ` · via ${dep.trigger}` : ""}
        </span>
      </span>,
    ],
    [
      "Source",
      <span key="src" className="font-mono text-xs">
        {dep.branch}@{dep.commit_sha.slice(0, 7)}
        {dep.commit_meta?.author && (
          <span className="text-muted-foreground"> · {dep.commit_meta.author}</span>
        )}
      </span>,
    ],
  ];

  return (
    <>
    <div className="mb-4 grid gap-2 sm:grid-cols-2">
      <div className="rounded-md border border-border p-3">
        <div className="mb-1 text-[10px] uppercase tracking-wide text-muted-foreground">
          Production deployment
        </div>
        {mainUrl ? (
          <div className="flex items-center gap-2">
            <Globe className="h-4 w-4 shrink-0 text-muted-foreground" />
            <a
              href={mainUrl.startsWith("http") ? mainUrl : `https://${mainUrl}`}
              target="_blank"
              rel="noreferrer"
              className="truncate font-mono text-sm hover:text-foreground"
            >
              {mainUrl.replace(/^https?:\/\//, "")}
            </a>
            <button
              onClick={() => copy(mainUrl)}
              className="shrink-0 text-muted-foreground hover:text-foreground"
              title="Copy URL"
            >
              {copied ? "✓" : "⧉"}
            </button>
          </div>
        ) : (
          <p className="text-xs text-muted-foreground">No public URL yet — still building.</p>
        )}
      </div>
      <div className="rounded-md border border-border p-3">
        {rows.map(([label, value]) => (
          <div key={label} className="flex items-baseline gap-3 py-0.5 text-xs">
            <span className="w-14 shrink-0 uppercase tracking-wide text-muted-foreground text-[10px]">
              {label}
            </span>
            <span className="min-w-0">{value}</span>
          </div>
        ))}
        {extraUrls.length > 0 && (
          <div className="mt-1 flex flex-wrap gap-x-3 gap-y-0.5">
            {extraUrls.map((u) => (
              <a
                key={u}
                href={`https://${u}`}
                target="_blank"
                rel="noreferrer"
                className="font-mono text-xs text-muted-foreground hover:text-foreground"
              >
                {u}
              </a>
            ))}
          </div>
        )}
      </div>
    </div>
    {mainUrl && <Preview url={mainUrl} />}
    <DeployStats id={dep.id} />
    </>
  );
}

function Preview({ url }: { url: string }) {
  const [state, setState] = useState<"probing" | "ok" | "dead">("probing");
  const href = url.startsWith("http") ? url : `https://${url}`;
  useEffect(() => {
    let live = true;
    fetch(href, { mode: "no-cors", signal: AbortSignal.timeout(5000) })
      .then(() => live && setState("ok"))
      .catch(() => live && setState("dead"));
    return () => {
      live = false;
    };
  }, [href]);
  return (
    <div className="mb-4 overflow-hidden rounded-lg border border-border">
      <div className="flex items-center justify-between border-b border-border px-3 py-2">
        <span className="text-xs text-muted-foreground">Preview — the live site, embedded</span>
        <a
          href={href}
          target="_blank"
          rel="noreferrer"
          className="text-xs text-muted-foreground hover:text-foreground"
        >
          Open ↗
        </a>
      </div>
      {state === "dead" ? (
        <div className="flex h-[200px] flex-col items-center justify-center gap-2 bg-muted/20">
          <Globe className="h-5 w-5 text-muted-foreground" />
          <p className="text-sm text-muted-foreground">
            Preview unavailable — <span className="font-mono">{url.replace(/^https?:\/\//, "")}</span>{" "}
            isn't reachable from this network.
          </p>
        </div>
      ) : (
        <iframe
          src={href}
          title="Deployment preview"
          loading="lazy"
          sandbox="allow-scripts allow-forms allow-popups allow-modals"
          className="h-[420px] w-full bg-muted/20"
        />
      )}
    </div>
  );
}

function DeployStats({ id }: { id: string }) {
  const [samples, setSamples] =
    useState<Awaited<ReturnType<typeof api.deploymentMetrics>>["samples"]>([]);
  useEffect(() => {
    api
      .deploymentMetrics(id)
      .then((m) => setSamples(m.samples))
      .catch(() => {});
  }, [id]);
  if (samples.length < 2) return null;
  const first = samples[0];
  const last = samples[samples.length - 1];
  const avgCpu = samples.reduce((a, s) => a + (s.cpu_pct ?? 0), 0) / samples.length;
  const peakMem = Math.max(...samples.map((s) => s.mem_used ?? 0));
  const rx = Math.max(0, (last.net_rx ?? 0) - (first.net_rx ?? 0));
  const tx = Math.max(0, (last.net_tx ?? 0) - (first.net_tx ?? 0));
  const spanH =
    (new Date(last.ts).getTime() - new Date(first.ts).getTime()) / 3600000;
  const stats: [string, string][] = [
    ["Transfer", `↓${fmtBytes(rx)} ↑${fmtBytes(tx)}`],
    ["Avg CPU", `${avgCpu.toFixed(1)}%`],
    ["Peak memory", fmtBytes(peakMem)],
    ["Sampled", spanH < 1 ? `${Math.round(spanH * 60)}m` : `${spanH.toFixed(1)}h`],
  ];
  return (
    <div className="mb-4 grid grid-cols-2 gap-2 sm:grid-cols-4">
      {stats.map(([label, value]) => (
        <div key={label} className="rounded-md border border-border px-3 py-2">
          <div className="text-[10px] uppercase tracking-wide text-muted-foreground">{label}</div>
          <div className="font-mono text-sm tabular-nums">{value}</div>
        </div>
      ))}
    </div>
  );
}

// The standard tags a link preview consumes — mirrors Vercel's OG tab.
const OG_TAGS: [string, string, string][] = [
  ["og:title", "Title shown in link previews — keep it under ~60 chars.", `<meta property="og:title" content="Page title">`],
  ["og:description", "Summary shown under the title.", `<meta property="og:description" content="Short summary">`],
  ["og:image", "Preview image URL — ≤5MB, JPG/PNG/WEBP/GIF.", `<meta property="og:image" content="https://<host>/og.png">`],
  ["og:url", "Canonical URL of the page.", `<meta property="og:url" content="https://<host>/">`],
  ["og:type", "Object type — usually `website`.", `<meta property="og:type" content="website">`],
  ["twitter:card", "`summary` or `summary_large_image`.", `<meta name="twitter:card" content="summary_large_image">`],
  ["twitter:title", "Twitter title — og:title is the fallback.", `<meta name="twitter:title" content="Page title">`],
  ["twitter:description", "Twitter description — og:description is the fallback.", `<meta name="twitter:description" content="Short summary">`],
];

function ogValue(report: NonNullable<Awaited<ReturnType<typeof api.deploymentOg>>["report"]>, tag: string) {
  if (tag.startsWith("twitter:")) return report.twitter[tag.slice(8)] ?? null;
  if (tag.startsWith("og:")) return report.og[tag.slice(3)] ?? null;
  return null;
}

function OgPanel({ id }: { id: string }) {
  const [open, setOpen] = useState(false);
  const [path, setPath] = useState("/");
  const [data, setData] = useState<Awaited<ReturnType<typeof api.deploymentOg>> | null>(null);
  const [busy, setBusy] = useState(false);
  const [copiedTag, setCopiedTag] = useState<string | null>(null);

  function fetchOg(p: string) {
    setBusy(true);
    api
      .deploymentOg(id, p)
      .then(setData)
      .catch((e) => setData({ ok: false, url: "", reason: e.message }))
      .finally(() => setBusy(false));
  }

  function copySnippet(tag: string, snippet: string, value: string | null) {
    const line = value
      ? snippet.replace(/content="[^"]*"/, `content="${value.replace(/"/g, "&quot;")}"`)
      : snippet;
    navigator.clipboard
      .writeText(line)
      .then(() => {
        setCopiedTag(tag);
        setTimeout(() => setCopiedTag(null), 1500);
      })
      .catch(() => {});
  }

  return (
    <div className="mb-4 overflow-hidden rounded-lg border border-border">
      <button
        onClick={() => {
          const next = !open;
          setOpen(next);
          if (next && !data && !busy) fetchOg(path);
        }}
        className="flex w-full items-center justify-between px-3 py-2.5 text-left transition-colors hover:bg-accent/50"
      >
        <span className="flex items-center gap-2 text-xs font-medium text-muted-foreground">
          <Globe className="h-3.5 w-3.5" />
          Open Graph — how this deployment previews when linked
        </span>
        <span className="text-xs text-muted-foreground">{open ? "−" : "+"}</span>
      </button>
      {open && (
        <div className="border-t border-border">
          <div className="flex items-center gap-2 border-b border-border px-3 py-2">
            <span className="text-[10px] uppercase tracking-wide text-muted-foreground">
              pathname
            </span>
            <input
              value={path}
              onChange={(e) => setPath(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && fetchOg(path)}
              placeholder="/"
              className="w-40 bg-transparent font-mono text-xs text-foreground placeholder:text-muted-foreground/50 focus:outline-none"
            />
            <button
              onClick={() => fetchOg(path)}
              className="text-xs text-muted-foreground transition-colors hover:text-foreground"
            >
              {busy ? "fetching…" : "fetch"}
            </button>
            {data?.ok && (
              <span className="ml-auto truncate font-mono text-[10px] text-muted-foreground/70">
                {data.url}
              </span>
            )}
          </div>
          {data && !data.ok && (
            <p className="px-3 py-3 text-xs text-muted-foreground">
              Couldn't fetch {data.url || "the deployment URL"}
              {data.reason ? ` — ${data.reason}` : ""}.
            </p>
          )}
          {data?.ok && data.report && (
            <div className="divide-y divide-border">
              {(data.report.title || data.report.description) && (
                <div className="px-3 py-2 text-xs text-muted-foreground">
                  {data.report.title && (
                    <span className="mr-3">
                      title <span className="text-foreground">{data.report.title}</span>
                    </span>
                  )}
                  {data.report.description && (
                    <span>
                      description{" "}
                      <span className="text-foreground">
                        {data.report.description.slice(0, 80)}
                      </span>
                    </span>
                  )}
                </div>
              )}
              {OG_TAGS.map(([tag, hint, snippet]) => {
                const value = ogValue(data.report!, tag);
                return (
                  <div key={tag} className="px-3 py-2">
                    <div className="flex items-center justify-between gap-2">
                      <span className="flex items-center gap-2">
                        <span
                          className={cn(
                            "h-1.5 w-1.5 rounded-full",
                            value ? "bg-emerald-500" : "bg-muted-foreground/40",
                          )}
                        />
                        <span className="font-mono text-xs">{tag}</span>
                      </span>
                      <span className="flex items-center gap-2">
                        {value ? (
                          <span className="max-w-64 truncate font-mono text-xs text-foreground">
                            {value}
                          </span>
                        ) : (
                          <span className="text-xs text-muted-foreground">not provided</span>
                        )}
                        <button
                          title={value ? "Copy meta tag" : "Copy suggested tag"}
                          onClick={() => copySnippet(tag, snippet, value)}
                          className="text-muted-foreground transition-colors hover:text-foreground"
                        >
                          {copiedTag === tag ? "✓" : "⧉"}
                        </button>
                      </span>
                    </div>
                    {!value && (
                      <p className="mt-1 pl-3.5 text-[11px] text-muted-foreground/70">{hint}</p>
                    )}
                  </div>
                );
              })}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

const FILE_TYPES: [RegExp, string][] = [
  [/\.html?$/i, "HTML"],
  [/\.css$/i, "CSS"],
  [/\.m?js$/i, "JavaScript"],
  [/\.map$/i, "Source map"],
  [/\.(png|jpe?g|gif|svg|webp|ico|avif)$/i, "Image"],
  [/\.(woff2?|ttf|otf|eot)$/i, "Font"],
  [/\.(mp4|webm|mov|mp3|wav|ogg)$/i, "Media"],
  [/\.json$/i, "JSON"],
  [/\.(txt|md)$/i, "Text"],
  [/\.wasm$/i, "WASM"],
  [/\.pdf$/i, "PDF"],
];

function fileType(path: string): string {
  return FILE_TYPES.find(([re]) => re.test(path))?.[1] ?? "File";
}

// Deployed-file inventory — Vercel's Resources tab. Lazy: the listing is
// only fetched when the panel opens.
function FilesPanel({ id }: { id: string }) {
  const [open, setOpen] = useState(false);
  const [data, setData] = useState<Awaited<ReturnType<typeof api.deploymentFiles>> | null>(null);
  const [filter, setFilter] = useState("");
  const files = (data?.files ?? []).filter(
    (f) => !filter || f.path.toLowerCase().includes(filter.toLowerCase()),
  );
  const total = (data?.files ?? []).reduce((n, f) => n + f.size, 0);

  return (
    <div className="mb-4 overflow-hidden rounded-lg border border-border">
      <button
        onClick={() => {
          const next = !open;
          setOpen(next);
          if (next && !data) api.deploymentFiles(id).then(setData).catch(() => {});
        }}
        className="flex w-full items-center justify-between px-3 py-2.5 text-left transition-colors hover:bg-accent/50"
      >
        <span className="flex items-center gap-2 text-xs font-medium text-muted-foreground">
          <FolderTree className="h-3.5 w-3.5" />
          Files — the static output this deployment serves
        </span>
        <span className="text-xs text-muted-foreground">{open ? "−" : "+"}</span>
      </button>
      {open && (
        <div className="border-t border-border">
          <div className="flex items-center gap-2 border-b border-border px-3 py-2">
            <input
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              placeholder="filter paths…"
              className="w-44 bg-transparent font-mono text-xs text-foreground placeholder:text-muted-foreground/50 focus:outline-none"
            />
            {data?.ok && (
              <span className="ml-auto font-mono text-[10px] text-muted-foreground/70">
                {data.files?.length ?? 0} file{(data.files?.length ?? 0) === 1 ? "" : "s"} ·{" "}
                {fmtBytes(total)}
              </span>
            )}
          </div>
          {data && !data.ok && (
            <p className="px-3 py-3 text-xs text-muted-foreground">
              {data.reason === "static_output_missing"
                ? "Static output expected — the published artifact is missing from storage."
                : "No file payload — server deployments keep their artifact inside the container image."}
            </p>
          )}
          {!data && <p className="px-3 py-3 text-xs text-muted-foreground">loading…</p>}
          {data?.ok && files.length === 0 && (
            <p className="px-3 py-3 text-xs text-muted-foreground">no files match.</p>
          )}
          {data?.ok && files.length > 0 && (
            <div className="divide-y divide-border">
              {files.map((f) => (
                <div key={f.path} className="flex items-center gap-3 px-3 py-1.5 text-xs">
                  <span className="min-w-0 flex-1 truncate font-mono text-foreground">
                    {f.path}
                  </span>
                  <span className="w-20 shrink-0 text-muted-foreground">{fileType(f.path)}</span>
                  <span className="w-16 shrink-0 text-right font-mono text-muted-foreground">
                    {fmtBytes(f.size)}
                  </span>
                </div>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function Stepper({ dep }: { dep: Deployment }) {
  const idx = Math.max(0, STEPS.indexOf(dep.status));
  const done = dep.conclusion === "succeeded";
  const failed = dep.conclusion != null && !done;
  return (
    <div className="mb-4 flex items-center gap-0">
      {STEPS.map((s, i) => {
        const state = done
          ? "done"
          : failed
            ? i < idx
              ? "done"
              : i === idx
                ? "failed"
                : "pending"
            : i < idx
              ? "done"
              : i === idx
                ? "active"
                : "pending";
        return (
          <div key={s} className="flex items-center">
            {i > 0 && (
              <div
                className={cn(
                  "h-px w-4 sm:w-10",
                  state === "done" || state === "active" || state === "failed"
                    ? "bg-zinc-500"
                    : "bg-border",
                )}
              />
            )}
            <div className="flex items-center gap-2 px-2">
              <span
                className={cn(
                  "flex h-4 w-4 items-center justify-center rounded-full text-[9px] font-medium",
                  state === "done" && "bg-emerald-500/15 text-emerald-400",
                  state === "active" && "bg-brand/15 text-brand",
                  state === "failed" && "bg-red-500/15 text-red-400",
                  state === "pending" && "bg-muted text-muted-foreground",
                )}
              >
                {state === "done" ? "✓" : state === "failed" ? "✕" : i + 1}
              </span>
              <span
                className={cn(
                  "text-xs capitalize",
                  state === "pending" ? "text-muted-foreground/60" : "text-muted-foreground",
                  state === "active" && "text-foreground",
                )}
              >
                {s}
              </span>
            </div>
          </div>
        );
      })}
    </div>
  );
}

export default function DeploymentPage() {
  const { id = "" } = useParams();
  const [dep, setDep] = useState<Deployment | null>(null);
  const [lines, setLines] = useState<string[]>([]);
  const [filter, setFilter] = useState("");
  const [copied, setCopied] = useState(false);
  const [following, setFollowing] = useState(true);
  const [stepOpen, setStepOpen] = useState<Record<number, boolean>>({});
  const [aliasAction, setAliasAction] = useState<{
    kind: "rollback" | "promote";
    phase: "confirm" | "busy" | "done" | "error";
    error?: string;
  } | null>(null);
  const [, setTick] = useState(0);
  const logRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    api.deployment(id).then(setDep).catch(() => {});

    // Logs come from two sources that overlap: the REST tail and the
    // SSE stream (which replays the tail as history before live lines,
    // and replays it again on every reconnect). Dedupe against what's
    // already rendered — the rfc3339 prefix makes each line unique, and
    // using prev (not an effect-local set) survives StrictMode remounts.
    const add = (batch: string[]) =>
      setLines((prev) => {
        const existing = new Set(prev);
        const fresh = batch.filter((l) => !existing.has(l));
        return fresh.length ? [...prev, ...fresh].slice(-4000) : prev;
      });
    api.deploymentLogs(id)
      .then((t) => add(t.split("\n").filter(Boolean)))
      .catch(() => {});

    const es = deploymentLogsStream(id);
    es.onmessage = (ev) => add([ev.data as string]);
    return () => es.close();
  }, [id]);

  // Re-fetch the deployment while it is running so status/conclusion settle live.
  const active = dep ? isRunning(dep.status, dep.conclusion) : false;
  const errMsg =
    typeof dep?.error === "string" ? dep.error : (dep?.error?.message ?? null);
  useEffect(() => {
    if (!active) return;
    const poll = setInterval(
      () => api.deployment(id).then(setDep).catch(() => {}),
      3000,
    );
    const tick = setInterval(() => setTick((t) => t + 1), 1000);
    return () => {
      clearInterval(poll);
      clearInterval(tick);
    };
  }, [id, active]);

  useEffect(() => {
    if (following) logRef.current?.scrollTo(0, logRef.current.scrollHeight);
  }, [lines, following]);

  function onLogScroll() {
    const el = logRef.current;
    if (!el) return;
    setFollowing(el.scrollHeight - el.scrollTop - el.clientHeight < 40);
  }

  const sections = groupLogLines(lines);
  const lastSection = sections.length - 1;
  // Completed phases collapse (Vercel behavior); a click toggles and
  // sticks, so an expanded section survives the next phase marker.
  const sectionOpen = (i: number) => stepOpen[i] ?? i === lastSection;

  function copyLogs() {
    navigator.clipboard
      .writeText(lines.join("\n"))
      .then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      })
      .catch(() => {});
  }

  function downloadLogs() {
    const blob = new Blob([lines.join("\n")], { type: "text/plain" });
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = `deploy-${id.slice(0, 8)}.log`;
    a.click();
    URL.revokeObjectURL(a.href);
  }

  function envLabel(d: Deployment) {
    return d.environment_id === "prod" ? "Production" : `the ${d.environment_id} environment`;
  }

  const isCurrent = !!dep && dep.env_state?.current_deployment_id === dep.id;
  const canRollback = !!dep && dep.env_state?.previous_deployment_id === dep.id;
  const canPromote =
    !!dep &&
    dep.conclusion === "succeeded" &&
    dep.prod_state?.current_deployment_id !== dep.id;
  const canSkip = !!dep && !dep.conclusion && ["pending", "queued"].includes(dep.status);

  function copyText(text: string) {
    navigator.clipboard.writeText(text).catch(() => {});
  }

  function doAliasAction(kind: "rollback" | "promote") {
    if (!dep) return;
    setAliasAction({ kind, phase: "busy" });
    const call =
      kind === "rollback"
        ? api.rollback(dep.project_id, dep.environment_id).then(() => {})
        : api.promote(dep.id).then(() => {});
    call
      .then(() => {
        setAliasAction({ kind, phase: "done" });
        api.deployment(id).then(setDep).catch(() => {});
      })
      .catch((e) =>
        setAliasAction({ kind, phase: "error", error: e.message ?? "unknown error" }),
      );
  }

  function menuItems(d: Deployment): MenuItem[] {
    const items: MenuItem[] = [];
    if (canRollback) {
      items.push({
        label: "Instant rollback",
        icon: <History className="h-3.5 w-3.5" />,
        onSelect: () => setAliasAction({ kind: "rollback", phase: "confirm" }),
      });
    }
    if (canPromote) {
      items.push({
        label: "Promote to production",
        icon: <ArrowUpToLine className="h-3.5 w-3.5" />,
        onSelect: () => setAliasAction({ kind: "promote", phase: "confirm" }),
      });
    }
    items.push(
      {
        label: "Copy deployment URL",
        icon: <Copy className="h-3.5 w-3.5" />,
        disabled: !d.url,
        onSelect: () => d.url && copyText(d.url),
      },
      {
        label: "Copy deployment ID",
        icon: <Copy className="h-3.5 w-3.5" />,
        onSelect: () => copyText(d.id),
      },
      {
        label: "Redeploy",
        icon: <Repeat className="h-3.5 w-3.5" />,
        onSelect: () =>
          api.redeploy(d.id).then((n) => (location.href = `/deployments/${n.id}`)),
      },
      {
        label: "Refresh status",
        icon: <RefreshCw className="h-3.5 w-3.5" />,
        onSelect: () =>
          api.reconcile(d.id).then(setDep).catch(() => api.deployment(d.id).then(setDep)),
      },
    );
    if (canSkip) {
      items.push({
        label: "Skip build",
        icon: <ListPlus className="h-3.5 w-3.5" />,
        onSelect: () => api.skip(d.id).then(() => api.deployment(d.id).then(setDep)),
      });
    }
    return items;
  }

  return (
    <div className="page-enter mx-auto flex min-h-[calc(100vh-3.5rem)] max-w-5xl flex-col px-4 py-6 sm:px-8">
      <div className="mb-4 flex items-center justify-between">
        <div className="flex min-w-0 items-center gap-3">
          {dep && <StatusDot status={dep.status} conclusion={dep.conclusion} computed={dep.computed_status} />}
          <h1 className="font-mono text-lg">
            {dep?.commit_sha.slice(0, 7) ?? id.slice(0, 7)}
          </h1>
          {dep && (
            <Badge variant={displayStatus(dep).variant}>
              {displayStatus(dep).label}
            </Badge>
          )}
          {dep && (
            <span className="min-w-0 truncate text-xs text-muted-foreground">
              {dep.branch}
              {dep.environment_id &&
                ` · ${dep.environment_id === "prod" ? "production" : dep.environment_id}`}
              {isCurrent && (
                <Badge variant="outline" className="ml-1.5 align-middle text-[10px]">
                  current
                </Badge>
              )}{" "}
              · {timeAgo(dep.created_at)}
              {active && ` · running ${elapsed(dep.created_at)}`}
              {!active && duration(dep.created_at, dep.concluded_at) &&
                ` · took ${duration(dep.created_at, dep.concluded_at)}`}
              {dep.commit_meta?.author && ` · ${dep.commit_meta.author}`}
            </span>
          )}
        </div>
        <div className="flex gap-2">
          {dep?.url && (
            <Button
              variant="outline"
              size="sm"
              onClick={() => window.open(dep.url, "_blank")}
            >
              Visit
            </Button>
          )}
          {active && (
            <Button
              variant="outline"
              size="sm"
              onClick={() => api.cancel(id).then(() => location.reload())}
            >
              Cancel
            </Button>
          )}
          {dep && <Menu title="Deployment actions" items={menuItems(dep)} />}
        </div>
      </div>
      {aliasAction && dep && (
        <div className="mb-4 flex flex-wrap items-center justify-between gap-3 rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2">
          <p className="text-xs text-amber-200/90">
            {aliasAction.phase === "confirm" &&
              aliasAction.kind === "rollback" &&
              `Instant rollback — ${envLabel(dep)} will serve ${dep.commit_sha.slice(0, 7)} again, replacing the current deployment.`}
            {aliasAction.phase === "confirm" &&
              aliasAction.kind === "promote" &&
              `Promote to production — production will serve ${dep.commit_sha.slice(0, 7)}, replacing the current deployment.`}
            {aliasAction.phase === "busy" &&
              (aliasAction.kind === "rollback"
                ? `Rolling back ${envLabel(dep)}…`
                : "Promoting to production…")}
            {aliasAction.phase === "done" &&
              (aliasAction.kind === "rollback"
                ? `${envLabel(dep)} now serves this deployment.`
                : "Production now serves this deployment.")}
            {aliasAction.phase === "error" &&
              `${aliasAction.kind === "rollback" ? "Rollback" : "Promote"} failed — ${aliasAction.error}`}
          </p>
          {aliasAction.phase === "confirm" && (
            <span className="flex gap-2">
              <Button
                size="sm"
                variant="outline"
                onClick={() => doAliasAction(aliasAction.kind)}
              >
                {aliasAction.kind === "rollback" ? "Roll back" : "Promote"}
              </Button>
              <Button size="sm" variant="ghost" onClick={() => setAliasAction(null)}>
                Dismiss
              </Button>
            </span>
          )}
          {(aliasAction.phase === "done" || aliasAction.phase === "error") && (
            <Button size="sm" variant="ghost" onClick={() => setAliasAction(null)}>
              Dismiss
            </Button>
          )}
        </div>
      )}
      {dep && firstLine(dep.commit_meta?.message) && (
        <p className="mb-3 truncate text-sm text-muted-foreground">
          {firstLine(dep.commit_meta?.message)}
        </p>
      )}
      {dep && <DeployInfo dep={dep} active={active} />}
      {dep && <Stepper dep={dep} />}
      {dep?.conclusion === "succeeded" && <OgPanel id={id} />}
      {dep && <FilesPanel id={id} />}
      {dep?.conclusion === "succeeded" && dep.container_status === "running" && (
        <LiveStats id={id} />
      )}
      {errMsg && (
        <div className="mb-4 rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 font-mono text-xs text-red-400">
          {errMsg}
        </div>
      )}
      <div className="relative flex h-[45vh] min-h-64 flex-col overflow-hidden rounded-lg border border-border bg-black/40">
        <div className="flex items-center justify-between border-b border-border px-3 py-2">
          <span className="flex items-center gap-2 text-xs text-muted-foreground">
            {active ? (
              <>
                <span className="relative flex h-1.5 w-1.5">
                  <span className="absolute h-full w-full animate-ping rounded-full bg-emerald-500 opacity-60" />
                  <span className="relative h-1.5 w-1.5 rounded-full bg-emerald-500" />
                </span>
                live
              </>
            ) : (
              "build log"
            )}
          </span>
          <div className="flex items-center gap-3">
            <input
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              placeholder="filter…"
              className="w-32 bg-transparent text-xs text-muted-foreground placeholder:text-muted-foreground/50 focus:text-foreground focus:outline-none"
            />
            <button
              className="text-xs text-muted-foreground transition-colors hover:text-foreground"
              onClick={copyLogs}
            >
              {copied ? "copied" : "copy"}
            </button>
            <button
              className="text-xs text-muted-foreground transition-colors hover:text-foreground"
              onClick={downloadLogs}
            >
              download
            </button>
          </div>
        </div>
        <div
          ref={logRef}
          onScroll={onLogScroll}
          className="flex-1 overflow-auto p-4 font-mono text-xs leading-relaxed text-zinc-300"
        >
          {lines.length === 0 ? (
            <span className="text-muted-foreground">waiting for logs…</span>
          ) : filter ? (
            lines.map((l, i) =>
              logStream(l) === "step" ||
              !logText(l)
                .replace(/\x1b\[[0-9;?]*[A-Za-z]/g, "")
                .toLowerCase()
                .includes(filter.toLowerCase()) ? null : (
                <div key={i} className="whitespace-pre-wrap break-all">
                  <Ansi text={logText(l)} />
                </div>
              ),
            )
          ) : (
            sections.map((s, si) => (
              <div key={si}>
                {s.step !== null && (
                  <button
                    onClick={() => setStepOpen((o) => ({ ...o, [si]: !sectionOpen(si) }))}
                    className="mt-1 flex w-full items-center gap-2 border-t border-border/60 py-1.5 text-left"
                  >
                    <span className="font-sans text-xs font-medium text-muted-foreground">
                      {stepTitle(s.step)}
                    </span>
                    <span className="text-[10px] text-muted-foreground/50">
                      {s.lines.length} line{s.lines.length === 1 ? "" : "s"}
                    </span>
                    <span className="ml-auto font-sans text-xs text-muted-foreground">
                      {sectionOpen(si) ? "−" : "+"}
                    </span>
                  </button>
                )}
                {(s.step === null || sectionOpen(si)) &&
                  s.lines.map((l, i) => (
                    <div key={i} className="whitespace-pre-wrap break-all">
                      <Ansi text={logText(l)} />
                    </div>
                  ))}
              </div>
            ))
          )}
        </div>
        {!following && (
          <button
            onClick={() => setFollowing(true)}
            className="absolute bottom-3 right-3 rounded-md border border-border bg-card px-2.5 py-1 text-xs text-muted-foreground shadow-lg transition-colors hover:text-foreground"
          >
            ↓ latest
          </button>
        )}
      </div>
    </div>
  );
}
