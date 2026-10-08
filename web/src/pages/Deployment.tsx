import { api, deploymentLogsStream, type Deployment } from "@/lib/api";
import { Badge, Button, StatusDot, statusVariant, isRunning } from "@/components/ui";
import { Ansi } from "@/lib/ansi";
import { cn, duration, elapsed, firstLine, timeAgo } from "@/lib/utils";
import { useEffect, useRef, useState } from "react";
import { useParams } from "react-router-dom";

const STEPS = ["prepare", "deploy", "finalize", "live"];

// Log lines arrive as "<rfc3339>\t<stream>\t<text>"; render the text.
const logText = (l: string) => {
  const i = l.indexOf("\t");
  const j = i < 0 ? -1 : l.indexOf("\t", i + 1);
  return j < 0 ? l : l.slice(j + 1);
};

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
                  "h-px w-10",
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
                  state === "active" && "bg-amber-400/15 text-amber-400",
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
  const [, setTick] = useState(0);
  const logRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    api.deployment(id).then(setDep).catch(() => {});
    api.deploymentLogs(id).then((t) => setLines(t.split("\n").filter(Boolean)));

    const es = deploymentLogsStream(id);
    es.onmessage = (ev) => setLines((prev) => [...prev.slice(-4000), ev.data as string]);
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

  return (
    <div className="page-enter mx-auto flex h-[calc(100vh-3.5rem)] max-w-5xl flex-col px-4 py-6 sm:px-8">
      <div className="mb-4 flex items-center justify-between">
        <div className="flex items-center gap-3">
          {dep && <StatusDot status={dep.status} conclusion={dep.conclusion} />}
          <h1 className="font-mono text-lg">
            {dep?.commit_sha.slice(0, 7) ?? id.slice(0, 7)}
          </h1>
          {dep && (
            <Badge variant={statusVariant(dep.status, dep.conclusion)}>
              {dep.conclusion ?? dep.status}
            </Badge>
          )}
          {dep && (
            <span className="truncate text-xs text-muted-foreground">
              {dep.branch}
              {dep.environment_id && ` · ${dep.environment_id}`} · {timeAgo(dep.created_at)}
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
          <Button
            variant="outline"
            size="sm"
            onClick={() => api.redeploy(id).then((d) => (location.href = `/deployments/${d.id}`))}
          >
            Redeploy
          </Button>
        </div>
      </div>
      {dep && firstLine(dep.commit_meta?.message) && (
        <p className="mb-3 truncate text-sm text-muted-foreground">
          {firstLine(dep.commit_meta?.message)}
        </p>
      )}
      {dep && <Stepper dep={dep} />}
      {errMsg && (
        <div className="mb-4 rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 font-mono text-xs text-red-400">
          {errMsg}
        </div>
      )}
      <div className="relative flex min-h-0 flex-1 flex-col overflow-hidden rounded-lg border border-border bg-black/40">
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
          ) : (
            lines.map((l, i) =>
              filter &&
              !logText(l)
                .replace(/\x1b\[[0-9;?]*[A-Za-z]/g, "")
                .toLowerCase()
                .includes(filter.toLowerCase()) ? null : (
                <div key={i} className="whitespace-pre-wrap break-all">
                  <Ansi text={logText(l)} />
                </div>
              ),
            )
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
