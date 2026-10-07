import {
  api,
  type AllowlistRule,
  type GitConnection,
  type Me,
  type RemoteNode,
} from "@/lib/api";
import { Badge, Button, Card, Input } from "@/components/ui";
import { useEffect, useState } from "react";

const selectCls =
  "h-9 w-full rounded-md border border-border bg-transparent px-3 text-sm focus:outline-none focus:ring-1 focus:ring-ring";

function GitProviders() {
  const [provider, setProvider] = useState("gitea");
  const [connections, setConnections] = useState<Record<string, GitConnection[]>>({});
  const [baseUrl, setBaseUrl] = useState("");
  const [workspace, setWorkspace] = useState("");
  const [token, setToken] = useState("");
  const [error, setError] = useState("");

  useEffect(() => {
    for (const p of ["gitea", "gitlab", "bitbucket"]) {
      api
        .gitConnections(p)
        .then((d) => setConnections((c) => ({ ...c, [p]: d.connections })))
        .catch(() => {});
    }
  }, []);

  async function connect(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    try {
      const body: { token: string; base_url?: string; workspace?: string } = { token };
      if (provider === "bitbucket") body.workspace = workspace;
      else if (baseUrl) body.base_url = baseUrl;
      const conn = await api.gitConnect(provider, body);
      setConnections((c) => ({ ...c, [provider]: [...(c[provider] ?? []), conn] }));
      setToken("");
      setBaseUrl("");
      setWorkspace("");
    } catch (err) {
      setError(err instanceof Error ? err.message : "connect failed");
    }
  }

  return (
    <Card className="p-5">
      <h2 className="mb-1 text-sm font-medium">Git providers</h2>
      <p className="mb-4 text-xs text-muted-foreground">
        Tokens for Gitea, GitLab, and Bitbucket. GitHub uses the App installation flow.
      </p>
      <form onSubmit={connect} className="mb-4 grid grid-cols-[110px_1fr_1fr_auto] gap-2">
        <select className={selectCls} value={provider} onChange={(e) => setProvider(e.target.value)}>
          <option value="gitea">Gitea</option>
          <option value="gitlab">GitLab</option>
          <option value="bitbucket">Bitbucket</option>
        </select>
        {provider === "bitbucket" ? (
          <Input
            placeholder="workspace slug"
            value={workspace}
            onChange={(e) => setWorkspace(e.target.value)}
          />
        ) : (
          <Input
            placeholder={provider === "gitlab" ? "https://gitlab.com (default)" : "instance URL"}
            value={baseUrl}
            onChange={(e) => setBaseUrl(e.target.value)}
          />
        )}
        <Input
          type="password"
          placeholder={provider === "bitbucket" ? "app password" : "access token"}
          value={token}
          onChange={(e) => setToken(e.target.value)}
          required
        />
        <Button type="submit" size="sm">
          Connect
        </Button>
      </form>
      {error && <p className="mb-3 text-xs text-destructive">{error}</p>}
      <div className="grid gap-2">
        {Object.entries(connections).flatMap(([p, conns]) =>
          conns.map((c) => (
            <div
              key={`${p}:${c.id}`}
              className="flex items-center justify-between rounded-md border border-border px-3 py-2"
            >
              <div className="text-sm">
                <span className="capitalize">{p}</span>
                <span className="ml-2 text-xs text-muted-foreground">
                  {c.username ?? ""} {c.base_url ?? ""}
                </span>
              </div>
            </div>
          )),
        )}
      </div>
    </Card>
  );
}

function Allowlist() {
  const [rules, setRules] = useState<AllowlistRule[]>([]);
  const [type, setType] = useState("email");
  const [value, setValue] = useState("");
  const [error, setError] = useState("");

  const load = () => api.allowlist().then((r) => setRules(r.rules)).catch((e) => setError(e.message));
  useEffect(() => {
    load();
  }, []);

  async function add(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    try {
      await api.addAllowlistRule(type, value);
      setValue("");
      load();
    } catch (err) {
      setError(err instanceof Error ? err.message : "failed");
    }
  }

  return (
    <Card className="p-5">
      <h2 className="mb-1 text-sm font-medium">Sign-up allowlist</h2>
      <p className="mb-4 text-xs text-muted-foreground">
        Empty list = open sign-up. Rules match emails, domains, or regex patterns.
      </p>
      <form onSubmit={add} className="mb-4 grid grid-cols-[110px_1fr_auto] gap-2">
        <select className={selectCls} value={type} onChange={(e) => setType(e.target.value)}>
          <option value="email">email</option>
          <option value="domain">domain</option>
          <option value="pattern">pattern</option>
        </select>
        <Input
          placeholder={type === "email" ? "user@example.com" : type === "domain" ? "example.com" : "^.*@acme\\.com$"}
          value={value}
          onChange={(e) => setValue(e.target.value)}
          required
        />
        <Button type="submit" size="sm">
          Add rule
        </Button>
      </form>
      {error && <p className="mb-3 text-xs text-destructive">{error}</p>}
      <div className="grid gap-2">
        {rules.map((r) => (
          <div
            key={r.id}
            className="flex items-center justify-between rounded-md border border-border px-3 py-2"
          >
            <div className="font-mono text-sm">
              <Badge variant="outline">{r.type}</Badge> <span className="ml-2">{r.value}</span>
            </div>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => api.deleteAllowlistRule(r.id).then(load).catch(() => {})}
            >
              Delete
            </Button>
          </div>
        ))}
      </div>
    </Card>
  );
}

function Nodes() {
  const [nodes, setNodes] = useState<RemoteNode[]>([]);
  const [name, setName] = useState("");
  const [host, setHost] = useState("");
  const [url, setUrl] = useState("");
  const [error, setError] = useState("");

  const load = () => api.nodes().then((r) => setNodes(r.nodes)).catch((e) => setError(e.message));
  useEffect(() => {
    load();
  }, []);

  async function add(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    try {
      await api.createNode({ name, host, docker_url: url });
      setName("");
      setHost("");
      setUrl("");
      load();
    } catch (err) {
      setError(err instanceof Error ? err.message : "failed");
    }
  }

  return (
    <Card className="p-5">
      <h2 className="mb-1 text-sm font-medium">Remote Docker nodes</h2>
      <p className="mb-4 text-xs text-muted-foreground">
        Deploy projects to remote Docker daemons. Containers publish a host port that Traefik
        routes to — the node must be network-reachable from this instance.
      </p>
      <form onSubmit={add} className="mb-4 grid grid-cols-[1fr_1fr_1.5fr_auto] gap-2">
        <Input placeholder="name" value={name} onChange={(e) => setName(e.target.value)} required />
        <Input
          placeholder="node.example.com"
          value={host}
          onChange={(e) => setHost(e.target.value)}
          required
        />
        <Input
          placeholder="tcp://node.example.com:2375"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          required
        />
        <Button type="submit" size="sm">
          Add node
        </Button>
      </form>
      {error && <p className="mb-3 text-xs text-destructive">{error}</p>}
      <div className="grid gap-2">
        {nodes.map((n) => (
          <div
            key={n.id}
            className="flex items-center justify-between rounded-md border border-border px-3 py-2"
          >
            <div className="text-sm">
              {n.name}
              <span className="ml-2 font-mono text-xs text-muted-foreground">
                {n.host} · {n.docker_url}
              </span>
            </div>
            <div className="flex items-center gap-2">
              <Badge variant={n.status === "online" ? "success" : "warning"}>{n.status}</Badge>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => api.checkNode(n.id).then(load).catch(() => {})}
              >
                Check
              </Button>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => api.deleteNode(n.id).then(load).catch(() => {})}
              >
                Delete
              </Button>
            </div>
          </div>
        ))}
      </div>
    </Card>
  );
}

export default function SettingsPage({ me }: { me: Me }) {
  return (
    <div className="mx-auto max-w-5xl p-8">
      <h1 className="mb-6 text-xl font-semibold">Settings</h1>
      <div className="grid gap-4">
        <GitProviders />
        {me.id === 1 && (
          <>
            <Allowlist />
            <Nodes />
          </>
        )}
      </div>
    </div>
  );
}
