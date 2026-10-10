import {
  api,
  type CreateProjectInput,
  type GitConnection,
  type Project,
  type Repo,
} from "@/lib/api";
import { Avatar, Button, Card, ComboBox, Input, Skeleton, StatusDot } from "@/components/ui";
import { duration, firstLine, timeAgo } from "@/lib/utils";
import { LayoutGrid, List, Plus, Search } from "lucide-react";
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
          const list: GhInstallation[] = d.installations ?? [];
          setInstallations(list);
          // Preselect the authed account — the common single-installation
          // case needs no extra click; keep a prior pick if still present.
          setInstallationId((cur) =>
            list.some((i) => i.id === cur) ? cur : (list[0]?.id ?? null),
          );
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

  return (
    <Card className="mb-6 p-5">
      <div className="mb-4 flex items-center gap-2.5">
        <div className="flex h-7 w-7 items-center justify-center rounded-md border border-brand/30 bg-brand/10">
          <Plus className="h-4 w-4 text-brand" />
        </div>
        <h2 className="text-sm font-medium">New project</h2>
      </div>
      <form onSubmit={create} className="grid gap-3">
        <div className="grid grid-cols-2 gap-3">
          <div className="grid grid-cols-4 gap-1 rounded-lg border border-border p-1">
            {PROVIDERS.map((p) => (
              <button
                key={p}
                type="button"
                onClick={() => setProvider(p)}
                className={`rounded-md px-2 py-1 text-xs transition-colors ${
                  provider === p
                    ? "border border-brand/40 bg-brand/15 font-medium text-brand"
                    : "border border-transparent text-muted-foreground hover:text-foreground"
                }`}
              >
                {p === "github" ? "GitHub" : p[0].toUpperCase() + p.slice(1)}
              </button>
            ))}
          </div>
          {provider === "github" ? (
            <ComboBox
              value={installationId == null ? "" : String(installationId)}
              onChange={(v) => setInstallationId(v ? Number(v) : null)}
              options={installations.map((i) => ({ value: String(i.id), label: i.account }))}
              placeholder="Installation…"
              searchPlaceholder="Search accounts…"
              emptyText="No installations — install the app below"
            />
          ) : (
            <ComboBox
              value={connId == null ? "" : String(connId)}
              onChange={(v) => setConnId(v ? Number(v) : null)}
              options={connections.map((c) => ({
                value: String(c.id),
                label: c.username ?? c.base_url ?? `connection ${c.id}`,
              }))}
              placeholder="Connection…"
              searchPlaceholder="Search connections…"
            />
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
              placeholder={
                provider === "bitbucket" ? "OAuth key:secret or access token" : "access token"
              }
              value={token}
              onChange={(e) => setToken(e.target.value)}
            />
            <Button type="button" variant="outline" size="sm" onClick={connect} disabled={!token}>
              Connect
            </Button>
          </div>
        )}

        <ComboBox
          value={repo}
          onChange={(v) => {
            setRepo(v);
            const r = repos.find((x) => x.full_name === v.split("|")[0]);
            if (r) setBranch(r.default_branch || "main");
          }}
          options={repos.map((r) => ({
            value: `${r.full_name}|${r.default_branch}`,
            label: r.full_name,
            hint: r.default_branch,
          }))}
          placeholder="Repository…"
          searchPlaceholder="Search repositories…"
          emptyText={installationId || connId ? "No repositories found" : "Pick an installation first"}
        />
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

export function ProjectCard({ p }: { p: Project }) {
  const dep = p.latest_deployment;
  return (
    <Link key={p.id} to={`/projects/${p.id}`} className="block">
      <Card className="flex h-full flex-col gap-3 p-4 transition-colors hover:border-brand/30 hover:bg-accent/50">
        <div className="flex items-center justify-between">
          <div className="flex min-w-0 items-center gap-2">
            <Avatar kind="project" id={p.id} name={p.name} hasAvatar={p.has_avatar} />
            <span className="truncate font-medium">{p.name}</span>
            {p.preset && (
              <span className="shrink-0 rounded border border-border px-1.5 py-0.5 font-mono text-[10px] uppercase tracking-wide text-muted-foreground">
                {p.preset}
              </span>
            )}
          </div>
          {dep && <StatusDot status={dep.status} conclusion={dep.conclusion} computed={dep.computed_status} />}
        </div>
        {p.url ? (
          <div className="truncate font-mono text-xs text-muted-foreground">
            {p.url.replace(/^https?:\/\//, "")}
          </div>
        ) : (
          <div className="text-xs text-muted-foreground/50">No domain yet</div>
        )}
        <div className="mt-auto border-t border-border pt-3">
          {dep ? (
            <>
              <div className="truncate text-xs">
                {firstLine(dep.commit_meta?.message) ?? dep.commit_sha.slice(0, 7)}
              </div>
              <div className="mt-1 flex items-center justify-between text-xs text-muted-foreground">
                <span className="truncate font-mono">
                  {p.repo_full_name} · {dep.branch}
                </span>
                <span className="ml-2 shrink-0">
                  {timeAgo(dep.created_at)}
                  {duration(dep.created_at, dep.concluded_at) &&
                    ` · ${duration(dep.created_at, dep.concluded_at)}`}
                </span>
              </div>
            </>
          ) : (
            <div className="text-xs text-muted-foreground/50">No deployments yet</div>
          )}
        </div>
      </Card>
    </Link>
  );
}

export default function Projects() {
  const [projects, setProjects] = useState<Project[] | null>(null);
  const [error, setError] = useState("");
  const [creating, setCreating] = useState(false);
  const [query, setQuery] = useState("");
  const [view, setView] = useState<"grid" | "list">("grid");

  useEffect(() => {
    api.projects().then((r) => setProjects(r.projects)).catch((e) => setError(e.message));
  }, []);

  const filtered = projects?.filter((p) => {
    const q = query.toLowerCase();
    return (
      !q ||
      p.name.toLowerCase().includes(q) ||
      p.repo_full_name.toLowerCase().includes(q) ||
      (p.url ?? "").toLowerCase().includes(q)
    );
  });

  return (
    <div className="page-enter mx-auto max-w-6xl p-4 sm:p-8">
      <div className="mb-6 flex items-center justify-between gap-3">
        <div>
          <h1 className="text-xl font-semibold">Projects</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            Deploy from git or uploads — each project gets preview URLs and rollbacks.
          </p>
        </div>
        <Button size="sm" onClick={() => setCreating(true)} disabled={creating}>
          New project
        </Button>
      </div>
      <div className="mb-5 flex items-center gap-2">
        <div className="relative flex-1">
          <Search className="absolute left-3 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
          <Input
            placeholder="Search projects…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            className="pl-9"
          />
        </div>
        <div className="flex overflow-hidden rounded-md border border-border">
          {(
            [
              ["grid", LayoutGrid],
              ["list", List],
            ] as const
          ).map(([v, Icon]) => (
            <button
              key={v}
              onClick={() => setView(v)}
              className={`p-2 transition-colors ${
                view === v ? "bg-accent text-foreground" : "text-muted-foreground hover:text-foreground"
              }`}
              title={`${v} view`}
            >
              <Icon className="h-3.5 w-3.5" />
            </button>
          ))}
        </div>
      </div>
      {creating && <NewProject onDone={() => setCreating(false)} />}
      {error && <p className="text-sm text-destructive">{error}</p>}
      {projects === null && !error && (
        <div className={view === "grid" ? "grid gap-3 sm:grid-cols-2 lg:grid-cols-3" : "grid gap-3"}>
          {[0, 1, 2].map((i) => (
            <Skeleton key={i} className="h-[104px]" />
          ))}
        </div>
      )}
      {filtered !== undefined && filtered !== null && filtered.length === 0 && !error && !creating && (
        <div className="rounded-lg border border-dashed border-border p-12 text-center">
          <p className="text-sm font-medium">
            {projects?.length === 0 ? "No projects yet" : "No matches"}
          </p>
          <p className="mt-1 text-xs text-muted-foreground">
            {projects?.length === 0
              ? "Connect a repository and ship your first deploy."
              : `Nothing matches "${query}".`}
          </p>
          {projects?.length === 0 && (
            <Button size="sm" className="mt-4" onClick={() => setCreating(true)}>
              New project
            </Button>
          )}
        </div>
      )}
      {view === "grid" ? (
        <div className="stagger grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {filtered?.map((p) => <ProjectCard key={p.id} p={p} />)}
        </div>
      ) : (
        <div className="stagger grid gap-2">
          {filtered?.map((p) => {
            const dep = p.latest_deployment;
            return (
              <Link key={p.id} to={`/projects/${p.id}`}>
                <Card className="flex items-center justify-between gap-4 p-3 transition-colors hover:border-muted-foreground/25 hover:bg-accent/50">
                  <div className="flex min-w-0 items-center gap-3">
                    {dep ? (
                      <StatusDot status={dep.status} conclusion={dep.conclusion} computed={dep.computed_status} />
                    ) : (
                      <StatusDot status="idle" />
                    )}
                    <Avatar kind="project" id={p.id} name={p.name} hasAvatar={p.has_avatar} />
                    <div className="min-w-0">
                      <div className="flex items-center gap-2">
                        <span className="truncate font-medium">{p.name}</span>
                        {p.preset && (
                          <span className="font-mono text-[10px] uppercase text-muted-foreground">
                            {p.preset}
                          </span>
                        )}
                      </div>
                      <div className="truncate font-mono text-xs text-muted-foreground">
                        {p.repo_full_name} · {p.repo_branch}
                      </div>
                    </div>
                  </div>
                  <div className="flex shrink-0 items-center gap-4 text-xs text-muted-foreground">
                    {dep && (
                      <span className="hidden truncate sm:block">
                        {firstLine(dep.commit_meta?.message)}
                      </span>
                    )}
                    <span className="shrink-0">
                      {dep ? timeAgo(dep.created_at) : timeAgo(p.created_at)}
                    </span>
                  </div>
                </Card>
              </Link>
            );
          })}
        </div>
      )}
    </div>
  );
}
