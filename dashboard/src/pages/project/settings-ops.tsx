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

type Env = { id: string; name: string; slug: string; branch?: string; status?: string };

export function Environments({ id, project }: { id: string; project: Project | null }) {
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
      <div className="mb-3 grid grid-cols-1 gap-2">
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
      <form onSubmit={add} className="grid grid-cols-1 sm:grid-cols-[1fr_1fr_auto] gap-2">
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

export function DockerBuild({ id, project }: { id: string; project: Project | null }) {
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
      <div className="grid grid-cols-1 gap-3">
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
      </div>
      <Err msg={error} />
      <CardFooter doc={`${DOCS}/presets.md`} docLabel="Learn more about Dockerfile builds">
        {msg && <span className="text-xs text-muted-foreground">{msg}</span>}
        <Button size="sm" onClick={save}>
          Save
        </Button>
      </CardFooter>
    </Card>
  );
}

export function HealthCheck({ id, project }: { id: string; project: Project | null }) {
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
      <Input
        placeholder="/healthz"
        value={path}
        onChange={(e) => setPath(e.target.value)}
        className="font-mono"
      />
      <Err msg={error} />
      <CardFooter doc={`${DOCS}/runway-json.md`} docLabel="Learn more about health checks">
        {msg && <span className="text-xs text-muted-foreground">{msg}</span>}
        <Button size="sm" onClick={save}>
          Save
        </Button>
      </CardFooter>
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

export function DeployRules({ id, project }: { id: string; project: Project | null }) {
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
      <div className="grid grid-cols-1 gap-3">
        <label className="flex items-center gap-2 text-xs">
          <input
            type="checkbox"
            checked={rules.auto_deploy !== false}
            onChange={(e) => set({ auto_deploy: e.target.checked })}
          />
          Auto-deploy on push
        </label>
        <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
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
      </div>
      <Err msg={error} />
      <CardFooter doc={`${DOCS}/runway-json.md`} docLabel="Learn more about deploy rules">
        {msg && <span className="text-xs text-muted-foreground">{msg}</span>}
        <Button size="sm" onClick={save}>
          Save rules
        </Button>
      </CardFooter>
    </Card>
  );
}

export function Retention({ id, project }: { id: string; project: Project | null }) {
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
      <Input
        placeholder="e.g. 5 — empty keeps all"
        value={count}
        onChange={(e) => setCount(e.target.value.replace(/\D/g, ""))}
        className="w-44"
      />
      <Err msg={error} />
      <CardFooter doc={`${DOCS}/runway-json.md`} docLabel="Learn more about retention">
        {msg && <span className="text-xs text-muted-foreground">{msg}</span>}
        <Button size="sm" onClick={save}>
          Save
        </Button>
      </CardFooter>
    </Card>
  );
}

export function StatusPageCard({ id, project }: { id: string; project: Project | null }) {
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
      </div>
      {enabled && (
        <div className="mt-3 space-y-2 text-xs">
          <p>
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
          <p className="flex items-center gap-2">
            <img src={`${window.location.origin}/api/v1/status/${slug || id}/badge`} alt="status badge" className="h-5" />
            <button
              type="button"
              className="font-mono text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
              onClick={() =>
                navigator.clipboard.writeText(
                  `[![status](${window.location.origin}/api/v1/status/${slug || id}/badge)](${url})`,
                )
              }
              title="Copy README markdown"
            >
              copy README badge
            </button>
          </p>
        </div>
      )}
      <Err msg={error} />
      <CardFooter doc={`${DOCS}/runway-json.md`} docLabel="Learn more about status pages">
        {enabled ? (
          <Button variant="outline" size="sm" onClick={() => save(false, slug)}>
            Disable
          </Button>
        ) : (
          <Button size="sm" onClick={() => save(true, slug)}>
            Enable
          </Button>
        )}
      </CardFooter>
    </Card>
  );
}

export function PreviewDomain({ id, project }: { id: string; project: Project | null }) {
  const [domain, setDomain] = useState("");
  const [msg, setMsg] = useState("");
  const [error, setError] = useState("");

  useEffect(() => {
    setDomain((project?.config?.preview_domain as string | undefined) ?? "");
  }, [project?.config]);

  const valid =
    domain.trim() === "" ||
    (/^[a-z0-9.-]+\.[a-z]{2,}$/i.test(domain.trim()) &&
      !domain.includes("://") &&
      !domain.includes("/") &&
      !/\s/.test(domain));

  async function save() {
    setError("");
    setMsg("");
    const d = domain.trim().toLowerCase();
    if (d !== "" && !valid) {
      setError("enter a bare domain like staging.example.com");
      return;
    }
    try {
      await api.patchProject(id, { config: { preview_domain: d } });
      setMsg(
        d === ""
          ? "Cleared — branch previews use the deploy domain again."
          : "Saved — new branch previews will live under " + d + ". Point *." + d + " at this instance (or assign via Cloudflare).",
      );
      setTimeout(() => setMsg(""), 4000);
    } catch (e) {
      setError(e instanceof Error ? e.message : "save failed");
    }
  }

  return (
    <Card className="p-4">
      <h3 className="mb-2 text-sm font-medium">Preview domain</h3>
      <p className="mb-3 text-xs text-muted-foreground">
        Staging base for branch previews: <code className="font-mono">&lt;project&gt;-branch-&lt;branch&gt;.{domain.trim() || "staging.example.com"}</code>.
        Environment and immutable URLs stay on the deploy domain.
      </p>
      <Input
        placeholder="staging.example.com (empty = deploy domain)"
        value={domain}
        onChange={(e) => setDomain(e.target.value)}
      />
      <Err msg={error} />
      <CardFooter doc={`${DOCS}/runway-json.md`} docLabel="Learn more about preview domains">
        {msg && <span className="text-xs text-muted-foreground">{msg}</span>}
        <Button size="sm" onClick={save}>
          Save
        </Button>
      </CardFooter>
    </Card>
  );
}

export function Firewall({ id, project }: { id: string; project: Project | null }) {
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
      <div className="grid grid-cols-1 gap-3">
        <textarea
          value={allow}
          onChange={(e) => setAllow(e.target.value)}
          placeholder={"IP allowlist — e.g.\n203.0.113.0/24\n198.51.100.7"}
          rows={3}
          className="w-full rounded-md border border-border bg-transparent px-3 py-2 font-mono text-sm placeholder:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-ring"
        />
        <div className="flex flex-wrap items-center gap-2">
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
        <Err msg={error} />
      </div>
      <CardFooter doc={`${DOCS}/runway-json.md`} docLabel="Learn more about the firewall">
        {msg && <span className="text-xs text-muted-foreground">{msg}</span>}
        <Button size="sm" onClick={save}>Save</Button>
      </CardFooter>
    </Card>
  );
}

export function Protection({ id, project }: { id: string; project: Project | null }) {
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
    <div className="grid grid-cols-1 gap-2 border-t border-border pt-3">
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
            autoComplete="new-password"
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
