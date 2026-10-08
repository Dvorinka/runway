import {
  api,
  type CreateProjectInput,
  type GitConnection,
  type Project,
  type Repo,
} from "@/lib/api";
import { Badge, Button, Card, Input, Skeleton, StatusDot } from "@/components/ui";
import { timeAgo } from "@/lib/utils";
import { useEffect, useState } from "react";
import { Link, useNavigate } from "react-router-dom";

const PROVIDERS = ["github", "gitea", "gitlab", "bitbucket"] as const;
type Provider = (typeof PROVIDERS)[number];

interface GhInstallation {
  id: number;
  account: string;
}
interface GhRepo {
  id: number;
  full_name: string;
  default_branch: string;
}

function NewProject({ onDone }: { onDone: () => void }) {
  const nav = useNavigate();
  const [provider, setProvider] = useState<Provider>("github");
  const [connections, setConnections] = useState<GitConnection[]>([]);
  const [connId, setConnId] = useState<number | null>(null);
  const [installations, setInstallations] = useState<GhInstallation[]>([]);
  const [ghInstallUrl, setGhInstallUrl] = useState<string | null>(null);
  const [ghConfigured, setGhConfigured] = useState(true);
  const [installationId, setInstallationId] = useState<number | null>(null);
  const [repos, setRepos] = useState<(Repo | GhRepo)[]>([]);
  const [repo, setRepo] = useState("");
  const [branch, setBranch] = useState("main");
  const [name, setName] = useState("");
  const [preset, setPreset] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  // Git provider connection form (non-github providers).
  const [baseUrl, setBaseUrl] = useState("");
  const [workspace, setWorkspace] = useState("");
  const [token, setToken] = useState("");

  useEffect(() => {
    setConnections([]);
    setConnId(null);
    setRepos([]);
    setRepo("");
    if (provider === "github") {
      fetch("/api/v1/github/installations", { credentials: "include" })
        .then((r) => r.json())
        .then((d) => {
          setInstallations(d.installations ?? []);
          setGhInstallUrl(d.install_url ?? null);
          setGhConfigured(d.configured !== false);
        })
        .catch(() => {});
    } else {
      api
        .gitConnections(provider)
        .then((d) => {
          setConnections(d.connections);
          if (d.connections[0]) setConnId(d.connections[0].id);
        })
        .catch(() => {});
    }
  }, [provider]);

  useEffect(() => {
    setRepos([]);
    setRepo("");
    if (provider === "github" && installationId) {
      fetch(`/api/v1/github/installations/${installationId}/repos`, { credentials: "include" })
        .then((r) => r.json())
        .then((d) => setRepos(d.repositories ?? []))
        .catch(() => {});
    } else if (provider !== "github" && connId) {
      api
        .gitRepos(provider, connId)
        .then((d) => setRepos(d.repositories))
        .catch((e) => setError(e.message));
    }
  }, [provider, installationId, connId]);

  async function connect(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    try {
      const body: { token: string; base_url?: string; workspace?: string } = { token };
      if (provider === "bitbucket") body.workspace = workspace;
      else if (baseUrl) body.base_url = baseUrl;
      const conn = await api.gitConnect(provider, body);
      setConnections((c) => [...c, conn]);
      setConnId(conn.id);
      setToken("");
    } catch (err) {
      setError(err instanceof Error ? err.message : "connect failed");
    }
  }

  async function create(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    setBusy(true);
    try {
      const [full, br] = repo.split("|");
      const body: CreateProjectInput = {
        name: name || full.split("/").pop() || full,
        repo_full_name: full,
        branch: branch || br || "main",
        provider,
      };
      if (provider === "github") {
        const r = repos.find((x) => x.full_name === full) as GhRepo | undefined;
        body.repo_id = r ? Number(r.id) : undefined;
        body.installation_id = installationId ?? undefined;
      } else if (connId) {
        body.connection_id = connId;
      }
      if (preset) body.preset = preset;
      const p = await api.createProject(body);
      nav(`/projects/${p.id}`);
    } catch (err) {
      setError(err instanceof Error ? err.message : "create failed");
      setBusy(false);
    }
  }

  const selectCls =
    "h-9 w-full rounded-md border border-border bg-transparent px-3 text-sm focus:outline-none focus:ring-1 focus:ring-ring";

  return (
    <Card className="mb-6 p-5">
      <h2 className="mb-4 text-sm font-medium">New project</h2>
      <form onSubmit={create} className="grid gap-3">
        <div className="grid grid-cols-2 gap-3">
          <select
            className={selectCls}
            value={provider}
            onChange={(e) => setProvider(e.target.value as Provider)}
          >
            {PROVIDERS.map((p) => (
              <option key={p} value={p}>
                {p === "github" ? "GitHub" : p[0].toUpperCase() + p.slice(1)}
              </option>
            ))}
          </select>
          {provider === "github" ? (
            <select
              className={selectCls}
              value={installationId ?? ""}
              onChange={(e) => setInstallationId(Number(e.target.value) || null)}
            >
              <option value="">Installation…</option>
              {installations.map((i) => (
                <option key={i.id} value={i.id}>
                  {i.account}
                </option>
              ))}
            </select>
          ) : (
            <select
              className={selectCls}
              value={connId ?? ""}
              onChange={(e) => setConnId(Number(e.target.value) || null)}
            >
              <option value="">Connection…</option>
              {connections.map((c) => (
                <option key={c.id} value={c.id}>
                  {c.username ?? c.base_url ?? `connection ${c.id}`}
                </option>
              ))}
            </select>
          )}
        </div>

        {provider === "github" && !ghConfigured && (
          <p className="rounded-md border border-border p-3 text-xs text-muted-foreground">
            No GitHub App registered on this instance — ask an admin to register one under{" "}
            <Link to="/settings" className="underline underline-offset-2">
              Settings → GitHub App
            </Link>
            .
          </p>
        )}
        {provider === "github" && ghConfigured && ghInstallUrl && (
          <p className="text-xs text-muted-foreground">
            Repository missing?{" "}
            <a
              className="underline underline-offset-2"
              href={ghInstallUrl}
              target="_blank"
              rel="noreferrer"
            >
              Install the app on another GitHub account or repo →
            </a>
          </p>
        )}
        {provider !== "github" && connections.length === 0 && (
          <div className="grid gap-2 rounded-md border border-border p-3">
            <p className="text-xs text-muted-foreground">
              Connect {provider === "bitbucket" ? "a Bitbucket workspace" : `a ${provider} instance`}:
            </p>
            {provider === "bitbucket" ? (
              <Input
                placeholder="workspace slug"
                value={workspace}
                onChange={(e) => setWorkspace(e.target.value)}
              />
            ) : (
              <Input
                placeholder={provider === "gitlab" ? "https://gitlab.com" : "https://git.example.com"}
                value={baseUrl}
                onChange={(e) => setBaseUrl(e.target.value)}
              />
            )}
            <Input
              type="password"
              placeholder={provider === "bitbucket" ? "app password" : "access token"}
              value={token}
              onChange={(e) => setToken(e.target.value)}
            />
            <Button type="button" variant="outline" size="sm" onClick={connect} disabled={!token}>
              Connect
            </Button>
          </div>
        )}

        <select
          className={selectCls}
          value={repo}
          onChange={(e) => {
            setRepo(e.target.value);
            const r = repos.find((x) => x.full_name === e.target.value.split("|")[0]);
            if (r) setBranch(r.default_branch || "main");
          }}
        >
          <option value="">Repository…</option>
          {repos.map((r) => (
            <option key={r.full_name} value={`${r.full_name}|${r.default_branch}`}>
              {r.full_name}
            </option>
          ))}
        </select>
        <div className="grid grid-cols-3 gap-3">
          <Input placeholder="Name (optional)" value={name} onChange={(e) => setName(e.target.value)} />
          <Input placeholder="Branch" value={branch} onChange={(e) => setBranch(e.target.value)} />
          <Input
            placeholder="Preset (auto)"
            value={preset}
            onChange={(e) => setPreset(e.target.value)}
          />
        </div>
        {error && <p className="text-xs text-destructive">{error}</p>}
        <div className="flex gap-2">
          <Button type="submit" size="sm" disabled={busy || !repo}>
            Create project
          </Button>
          <Button type="button" variant="ghost" size="sm" onClick={onDone}>
            Cancel
          </Button>
        </div>
      </form>
    </Card>
  );
}

export default function Projects() {
  const [projects, setProjects] = useState<Project[] | null>(null);
  const [error, setError] = useState("");
  const [creating, setCreating] = useState(false);

  useEffect(() => {
    api.projects().then((r) => setProjects(r.projects)).catch((e) => setError(e.message));
  }, []);

  return (
    <div className="page-enter mx-auto max-w-5xl p-8">
      <div className="mb-6 flex items-center justify-between">
        <h1 className="text-xl font-semibold">Projects</h1>
        <Button size="sm" onClick={() => setCreating(true)} disabled={creating}>
          New project
        </Button>
      </div>
      {creating && <NewProject onDone={() => setCreating(false)} />}
      {error && <p className="text-sm text-destructive">{error}</p>}
      {projects === null && !error && (
        <div className="grid gap-3">
          {[0, 1, 2].map((i) => (
            <Skeleton key={i} className="h-[68px]" />
          ))}
        </div>
      )}
      {projects !== null && projects.length === 0 && !error && !creating && (
        <div className="rounded-lg border border-dashed border-border p-12 text-center">
          <p className="text-sm font-medium">No projects yet</p>
          <p className="mt-1 text-xs text-muted-foreground">
            Connect a repository and ship your first deploy.
          </p>
          <Button size="sm" className="mt-4" onClick={() => setCreating(true)}>
            New project
          </Button>
        </div>
      )}
      <div className="stagger grid gap-3">
        {projects?.map((p) => (
          <Link key={p.id} to={`/projects/${p.id}`}>
            <Card className="flex items-center justify-between p-4 transition-colors hover:border-muted-foreground/25 hover:bg-accent/50">
              <div className="flex items-center gap-3">
                <StatusDot status={p.status} />
                <div>
                  <div className="font-medium">{p.name}</div>
                  <div className="font-mono text-xs text-muted-foreground">
                    {p.repo_full_name} · {p.repo_branch}
                  </div>
                </div>
              </div>
              <div className="flex items-center gap-4">
                <span className="text-xs text-muted-foreground">{timeAgo(p.created_at)}</span>
                <Badge variant={p.status === "active" ? "success" : "secondary"}>{p.status}</Badge>
              </div>
            </Card>
          </Link>
        ))}
      </div>
    </div>
  );
}
