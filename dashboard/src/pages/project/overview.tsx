import {
  api,
  type Deployment,
  type Domain,
  type Project,
} from "@/lib/api";
import {
  Badge,
  Button,
  Card,
  EmptyState,
  Skeleton,
  StatusDot,
  displayStatus,
} from "@/components/ui";
import { cn, timeAgo } from "@/lib/utils";
import { ArrowUpRight, Copy, ExternalLink, Rocket } from "lucide-react";
import { useEffect, useState } from "react";
import { Link } from "react-router-dom";

function liveUrlOf(dep: Deployment | null): string | null {
  if (!dep) return null;
  const scheme = dep.url?.split("://")[0] || "https";
  const host = dep.urls?.environment ?? dep.urls?.branch ?? dep.urls?.immutable;
  if (host?.startsWith("http")) return host;
  return host ? `${scheme}://${host}` : dep.url ?? null;
}

function hostOf(u: string): string {
  return u.replace(/^https?:\/\//, "");
}

export function Overview({ id, project }: { id: string; project: Project }) {
  const [deps, setDeps] = useState<Deployment[] | null>(null);
  const [domains, setDomains] = useState<Domain[]>([]);
  const [copied, setCopied] = useState("");
  const [deploying, setDeploying] = useState(false);
  const [error, setError] = useState("");

  const load = () => {
    api.deployments(id).then((r) => setDeps(r.deployments)).catch((e) => setError(e.message));
    api.domains(id).then((r) => setDomains(r.domains)).catch(() => {});
  };
  useEffect(load, [id]);

  const latest =
    deps?.find((d) => d.id === project.production_deployment_id) ??
    deps?.find((d) => d.conclusion === "succeeded") ??
    deps?.[0] ??
    null;
  const liveUrl = liveUrlOf(latest);
  const status = latest ? displayStatus(latest) : null;

  // Automatic aliases from the live deployment + custom domains.
  const aliases: { label: string; url: string }[] = [];
  if (latest) {
    const scheme = latest.url?.split("://")[0] || "https";
    const push = (label: string, host?: string | null) => {
      if (!host) return;
      const url = host.startsWith("http") ? host : `${scheme}://${host}`;
      if (!aliases.some((a) => a.url === url)) aliases.push({ label, url });
    };
    push("Environment", latest.urls?.environment);
    push("Branch", latest.urls?.branch);
    push("Immutable", latest.urls?.immutable);
  }

  function copy(url: string) {
    navigator.clipboard.writeText(url).then(() => {
      setCopied(url);
      setTimeout(() => setCopied(""), 1500);
    });
  }

  async function deploy() {
    setDeploying(true);
    setError("");
    try {
      await api.deploy(id);
      load();
    } catch (e) {
      setError(e instanceof Error ? e.message : "deploy failed");
    } finally {
      setDeploying(false);
    }
  }

  if (deps === null) {
    return (
      <div className="space-y-3">
        <Skeleton className="h-[380px]" />
        <Skeleton className="h-24" />
      </div>
    );
  }

  if (!latest) {
    return (
      <EmptyState
        icon={Rocket}
        title="Nothing deployed yet"
        hint="Deploy now or drop files on the Deployments tab to put this project live."
      />
    );
  }

  return (
    <div className="space-y-4">
      {liveUrl && (
        <div className="overflow-hidden rounded-lg border border-border">
          <div className="flex items-center justify-between border-b border-border bg-card px-4 py-2.5">
            <div className="flex min-w-0 items-center gap-2.5">
              <span
                className={cn(
                  "h-2 w-2 shrink-0 rounded-full",
                  status?.variant === "success"
                    ? "bg-success"
                    : status?.variant === "destructive"
                      ? "bg-destructive"
                      : "bg-muted-foreground/60",
                )}
              />
              <span className="truncate font-mono text-sm">{hostOf(liveUrl)}</span>
            </div>
            <div className="flex shrink-0 items-center gap-1">
              <Button
                variant="ghost"
                size="sm"
                onClick={() => copy(liveUrl)}
                title="Copy URL"
                aria-label="Copy URL"
              >
                {copied === liveUrl ? "✓" : <Copy className="h-3.5 w-3.5" />}
              </Button>
              <a
                href={liveUrl}
                target="_blank"
                rel="noreferrer"
                className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-xs text-muted-foreground transition-colors hover:bg-accent/60 hover:text-foreground"
              >
                Open <ArrowUpRight className="h-3.5 w-3.5" />
              </a>
            </div>
          </div>
          <iframe
            src={liveUrl}
            title="Live preview"
            loading="lazy"
            sandbox="allow-scripts allow-forms allow-popups allow-modals"
            className="h-[480px] w-full bg-muted/20"
          />
        </div>
      )}

      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <Card className="p-4">
          <div className="flex items-center justify-between">
            <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
              Production deployment
            </span>
            <StatusDot
              status={latest.status}
              conclusion={latest.conclusion}
              computed={latest.computed_status}
            />
          </div>
          <Link
            to={`/deployments/${latest.id}`}
            className="mt-2 block text-sm font-medium hover:underline"
          >
            {status?.label ?? latest.status}
            <span className="ml-2 font-mono text-xs text-muted-foreground">
              {latest.commit_sha.slice(0, 7)}
            </span>
          </Link>
          <dl className="mt-3 space-y-1.5 text-xs text-muted-foreground">
            <div className="flex justify-between gap-2">
              <dt>Source</dt>
              <dd className="truncate font-mono">
                {latest.branch}@{latest.commit_sha.slice(0, 7)}
              </dd>
            </div>
            <div className="flex justify-between gap-2">
              <dt>Via</dt>
              <dd className="truncate font-mono">{latest.trigger ?? "—"}</dd>
            </div>
            <div className="flex justify-between gap-2">
              <dt>Deployed</dt>
              <dd>{timeAgo(latest.created_at)}</dd>
            </div>
          </dl>
        </Card>

        <Card className="p-4">
          <div className="flex items-center justify-between">
            <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
              Domains
            </span>
            <Link
              to="?tab=domains"
              className="text-xs text-muted-foreground hover:text-foreground"
            >
              Manage
            </Link>
          </div>
          <div className="mt-2 space-y-1.5">
            {aliases.slice(0, 3).map((a) => (
              <a
                key={a.url}
                href={a.url}
                target="_blank"
                rel="noreferrer"
                className="flex items-center gap-2 truncate font-mono text-xs text-muted-foreground hover:text-foreground"
              >
                <Badge variant="secondary" className="shrink-0">
                  {a.label}
                </Badge>
                <span className="truncate">{hostOf(a.url)}</span>
              </a>
            ))}
            {domains.map((d) => (
              <a
                key={d.id}
                href={`https://${d.hostname}`}
                target="_blank"
                rel="noreferrer"
                className="flex items-center gap-2 truncate font-mono text-xs text-muted-foreground hover:text-foreground"
              >
                <Badge variant={d.status === "active" ? "success" : "warning"} className="shrink-0">
                  {d.status}
                </Badge>
                <span className="truncate">{d.hostname}</span>
              </a>
            ))}
            {aliases.length === 0 && domains.length === 0 && (
              <p className="text-xs text-muted-foreground">No domains yet.</p>
            )}
          </div>
        </Card>
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <Button size="sm" disabled={deploying} onClick={deploy}>
          {deploying ? "Deploying…" : "Deploy now"}
        </Button>
        <Link
          to={`/deployments/${latest.id}`}
          className="inline-flex items-center gap-1.5 rounded-md border border-border px-3 py-1.5 text-xs text-muted-foreground transition-colors hover:bg-accent/60 hover:text-foreground"
        >
          <ExternalLink className="h-3.5 w-3.5" /> Full deployment details
        </Link>
        {error && <span className="text-xs text-destructive">{error}</span>}
      </div>
    </div>
  );
}
