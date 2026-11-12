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

export function Preflight({ done, ready }: { done: Record<string, boolean>; ready: boolean }) {
  const items: { key: string; label: string; hint: string; to: string }[] = [
    { key: "deploy", label: "First deployment", hint: "drop files or hit Deploy now", to: "" },
    { key: "live", label: "Production live", hint: "a succeeded deployment serving traffic", to: "" },
    { key: "domain", label: "Custom domain", hint: "point DNS at this instance", to: "?tab=domains" },
    { key: "env", label: "Environment variable", hint: "store config securely", to: "?tab=environment" },
  ];
  const total = items.length;
  const count = items.filter((i) => done[i.key]).length;
  if (!ready || count === total) return null;
  return (
    <div className="mb-4 rounded-lg border border-border bg-card">
      <div className="flex items-center justify-between border-b border-border px-4 py-3">
        <span className="text-sm font-medium">Preflight checklist</span>
        <span className="rounded-full border border-border px-2 py-0.5 font-mono text-[11px] text-muted-foreground">
          {count}/{total}
        </span>
      </div>
      {items.map((i) => {
        const ok = done[i.key];
        return (
          <div key={i.key} className="flex items-center justify-between border-b border-border px-4 py-2.5 last:border-0">
            <div className="flex items-center gap-3">
              {ok ? (
                <CheckCircle2 className="h-4 w-4 text-success" />
              ) : (
                <Circle className="h-4 w-4 text-muted-foreground/50" />
              )}
              <span className={ok ? "text-sm text-muted-foreground" : "text-sm"}>{i.label}</span>
              {!ok && <span className="text-xs text-muted-foreground">· {i.hint}</span>}
            </div>
            {!ok && i.to && (
              <Link
                to={i.to}
                className="rounded-md border border-border px-2.5 py-1 text-xs text-muted-foreground transition-colors hover:text-foreground"
              >
                Open
              </Link>
            )}
          </div>
        );
      })}
    </div>
  );
}

export function Deployments({ id, project }: { id: string; project: Project | null }) {
  const nav = useNavigate();
  const [deployments, setDeployments] = useState<Deployment[] | null>(null);
  const [deploying, setDeploying] = useState(false);
  const [dragging, setDragging] = useState(false);
  const [uploading, setUploading] = useState(false);
  const [error, setError] = useState("");
  const [domainCount, setDomainCount] = useState<number | null>(null);
  const [envCount, setEnvCount] = useState<number | null>(null);

  useEffect(() => {
    const load = () =>
      api.deployments(id).then((r) => setDeployments(r.deployments)).catch((e) => setError(e.message));
    load();
    api.domains(id).then((r) => setDomainCount(r.domains.length)).catch(() => {});
    api.getEnv(id).then((r) => setEnvCount(r.env.length)).catch(() => {});
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
        <div className="pointer-events-none absolute inset-0 z-10 flex items-center justify-center rounded-lg border-2 border-dashed border-brand/60 bg-background/80">
          <div className="text-center">
            <p className="font-medium">Drop to deploy</p>
            <p className="mt-1 text-xs text-muted-foreground">
              HTML files, folders, or a .tar.gz — live in seconds
            </p>
          </div>
        </div>
      )}
      <div className="mb-4 flex flex-wrap items-center justify-between gap-2">
        <span className="text-xs text-muted-foreground">
          {uploading
            ? "Uploading…"
            : deployments && deployments.length > 0
              ? `${deployments.length} deployment${deployments.length === 1 ? "" : "s"} · tip: drop files here to deploy instantly`
              : "Tip: drop files here to deploy instantly"}
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
      {(() => {
        // The production alias target is the deployment actually live;
        // fall back to the newest succeeded one when no alias exists yet.
        const latest =
          deployments?.find((d) => d.id === project?.production_deployment_id) ??
          deployments?.find((d) => d.conclusion === "succeeded");
        const liveUrl = latest?.urls?.environment ?? latest?.urls?.immutable ?? null;
        if (!latest || !liveUrl) return null;
        return (
          <>
          <Link
            to={`/deployments/${latest.id}`}
            className="mb-2 block rounded-lg border border-border bg-card p-4 transition-colors hover:border-muted-foreground/30"
          >
            <div className="flex items-center justify-between gap-3">
              <div className="flex min-w-0 items-center gap-3">
                <StatusDot
                  status={latest.status}
                  conclusion={latest.conclusion}
                  computed={latest.computed_status}
                />
                <div className="min-w-0">
                  <div
                    className={cn(
                      "text-[10px] uppercase tracking-wide",
                      displayStatus(latest).variant === "success"
                        ? "text-success"
                        : displayStatus(latest).variant === "destructive"
                          ? "text-destructive"
                          : "text-muted-foreground",
                    )}
                  >
                    Production · {displayStatus(latest).label}
                  </div>
                  <div className="truncate font-mono text-sm">
                    {liveUrl.replace(/^https?:\/\//, "")}
                  </div>
                </div>
              </div>
              <div className="shrink-0 text-right text-xs text-muted-foreground">
                <div className="font-mono">{latest.commit_sha.slice(0, 7)}</div>
                <div>{timeAgo(latest.created_at)}</div>
              </div>
            </div>
          </Link>
          <div className="mb-2 flex items-center justify-between gap-3 rounded-lg border border-border bg-card px-4 py-2.5">
            <p className="text-xs text-muted-foreground">
              To update Production, drop files anywhere on this page or
              run <code className="rounded bg-accent px-1 py-0.5 font-mono text-[11px] text-foreground">runway deploy</code> from
              a linked directory.
            </p>
            <Link
              to="/deployments"
              className="flex shrink-0 items-center gap-1.5 rounded-md border border-border px-2.5 py-1 text-xs text-muted-foreground transition-colors hover:text-foreground"
            >
              <ListFilter size={13} />
              Deployments
            </Link>
          </div>
          </>
        );
      })()}
      <Preflight
        done={{
          deploy: (deployments?.length ?? 0) > 0,
          live:
            deployments?.some(
              (d) =>
                d.conclusion === "succeeded" &&
                (d.urls?.environment ?? d.urls?.immutable),
            ) ?? false,
          domain: (domainCount ?? 0) > 0,
          env: (envCount ?? 0) > 0,
        }}
        ready={deployments !== null && domainCount !== null && envCount !== null}
      />
      <div className="stagger grid grid-cols-1 gap-2">
        {deployments === null && [0, 1, 2].map((i) => <Skeleton key={i} className="h-[64px]" />)}
        {deployments?.map((d) => (
          <DeploymentRow key={d.id} d={d} />
        ))}
        {deployments !== null && deployments.length === 0 && (
          <EmptyState
            icon={Rocket}
            title="No deployments yet"
            hint="Hit Deploy now, or drop files here."
          />
        )}
      </div>
    </div>
  );
}

