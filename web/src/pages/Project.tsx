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
import { Badge, Button, Card, Input, statusVariant } from "@/components/ui";
import { useEffect, useRef, useState } from "react";
import { Link, useParams } from "react-router-dom";

const TABS = [
  "deployments",
  "environment",
  "cron",
  "redirects",
  "domains",
  "webhooks",
  "settings",
] as const;
type Tab = (typeof TABS)[number];

const selectCls =
  "h-9 w-full rounded-md border border-border bg-transparent px-3 text-sm focus:outline-none focus:ring-1 focus:ring-ring";

function Err({ msg }: { msg: string }) {
  return msg ? <p className="mt-2 text-xs text-destructive">{msg}</p> : null;
}

function Deployments({ id }: { id: string }) {
  const [deployments, setDeployments] = useState<Deployment[]>([]);
  const [error, setError] = useState("");

  useEffect(() => {
    const load = () =>
      api.deployments(id).then((r) => setDeployments(r.deployments)).catch((e) => setError(e.message));
    load();
    const es = projectEventsStream(id);
    es.onmessage = () => load();
    return () => es.close();
  }, [id]);

  return (
    <>
      <div className="mb-4 flex justify-end">
        <Button size="sm" onClick={() => api.deploy(id).catch((e) => setError(e.message))}>
          Deploy now
        </Button>
      </div>
      <Err msg={error} />
      <div className="grid gap-2">
        {deployments.map((d) => (
          <Link key={d.id} to={`/deployments/${d.id}`}>
            <Card className="flex items-center justify-between p-4 transition-colors hover:bg-accent">
              <div className="flex items-center gap-4">
                <Badge variant={statusVariant(d.status, d.conclusion)}>
                  {d.conclusion ?? d.status}
                </Badge>
                <div>
                  <div className="font-mono text-sm">{d.commit_sha.slice(0, 7)}</div>
                  <div className="text-xs text-muted-foreground">
                    {d.branch} · {d.environment_id}
                  </div>
                </div>
              </div>
              <div className="text-xs text-muted-foreground">
                {new Date(d.created_at).toLocaleString()}
              </div>
            </Card>
          </Link>
        ))}
      </div>
    </>
  );
}

function Environment({ id }: { id: string }) {
  const [vars, setVars] = useState<EnvVar[]>([]);
  const [key, setKey] = useState("");
  const [value, setValue] = useState("");
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

  return (
    <>
      <form onSubmit={add} className="mb-4 grid grid-cols-[1fr_1fr_auto] gap-2">
        <Input placeholder="KEY" value={key} onChange={(e) => setKey(e.target.value)} required />
        <Input placeholder="value" value={value} onChange={(e) => setValue(e.target.value)} />
        <Button type="submit" size="sm">
          Add
        </Button>
      </form>
      <Err msg={error} />
      <div className="grid gap-2">
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
      <div className="grid gap-2">
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
      <div className="grid gap-2">
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
      <div className="grid gap-2">
        {domains.map((d) => (
          <Card key={d.id} className="flex items-center justify-between p-3">
            <div className="font-mono text-sm">{d.hostname}</div>
            <div className="flex items-center gap-2">
              <Badge variant={d.status === "active" ? "success" : "warning"}>{d.status}</Badge>
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
      <div className="grid gap-2">
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

  return (
    <div className="grid gap-4">
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
      <Err msg={error} />
      {msg && <p className="text-xs text-muted-foreground">{msg}</p>}
    </div>
  );
}

export default function ProjectPage() {
  const { id = "" } = useParams();
  const [project, setProject] = useState<Project | null>(null);
  const [tab, setTab] = useState<Tab>("deployments");

  useEffect(() => {
    api.project(id).then(setProject).catch(() => {});
  }, [id]);

  return (
    <div className="mx-auto max-w-5xl p-8">
      <div className="mb-6 flex items-baseline justify-between">
        <div>
          <h1 className="text-xl font-semibold">{project?.name ?? "Project"}</h1>
          {project && (
            <p className="mt-1 text-xs text-muted-foreground">
              {project.repo_provider}:{project.repo_full_name} · {project.repo_branch}
              {project.url && (
                <>
                  {" · "}
                  <a href={project.url} target="_blank" className="underline">
                    {project.url}
                  </a>
                </>
              )}
            </p>
          )}
        </div>
      </div>
      <div className="mb-6 flex gap-4 border-b border-border">
        {TABS.map((t) => (
          <button
            key={t}
            onClick={() => setTab(t)}
            className={`-mb-px border-b-2 pb-2 text-sm capitalize ${
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
      {tab === "environment" && <Environment id={id} />}
      {tab === "cron" && <Cron id={id} />}
      {tab === "redirects" && <Redirects id={id} />}
      {tab === "domains" && <Domains id={id} />}
      {tab === "webhooks" && <Webhooks id={id} />}
      {tab === "settings" && <Settings id={id} project={project} />}
    </div>
  );
}
