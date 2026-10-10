import {
  api,
  type AuditEntry,
  type Me,
  type Project,
  type Storage,
  type TeamDetail,
  type TeamInvite,
  type TeamWebhook,
} from "@/lib/api";
import { Avatar, AvatarRow, Badge, Button, Card, ComboBox, Input } from "@/components/ui";
import { useCallback, useEffect, useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";

function Err({ msg }: { msg: string }) {
  return msg ? <p className="mb-3 text-xs text-destructive">{msg}</p> : null;
}

function Members({ team, myRole, me, reload }: {
  team: TeamDetail;
  myRole: string;
  me: Me;
  reload: () => void;
}) {
  const admin = myRole === "owner" || myRole === "admin";
  const [error, setError] = useState("");
  const act = (p: Promise<unknown>) => p.then(reload).catch((e) => setError(e.message));
  return (
    <Card className="p-5">
      <h2 className="mb-4 text-sm font-medium">Members</h2>
      <Err msg={error} />
      <div className="stagger grid gap-2">
        {team.members.map((m) => (
          <div
            key={m.user_id}
            className="flex items-center justify-between rounded-md border border-border px-3 py-2"
          >
            <div className="text-sm">
              {m.username}
              {m.user_id === me.id && (
                <span className="ml-2 text-xs text-muted-foreground">you</span>
              )}
            </div>
            <div className="flex items-center gap-2">
              {admin && m.user_id !== me.id ? (
                <ComboBox
                  className="w-28"
                  value={m.role}
                  onChange={(v) => act(api.updateMember(team.id, m.user_id, v))}
                  options={[
                    { value: "owner", label: "owner" },
                    { value: "admin", label: "admin" },
                    { value: "member", label: "member" },
                  ]}
                  placeholder="role…"
                />
              ) : (
                <Badge variant="secondary">{m.role}</Badge>
              )}
              {(admin || m.user_id === me.id) && (
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => act(api.removeMember(team.id, m.user_id))}
                >
                  {m.user_id === me.id ? "Leave" : "Remove"}
                </Button>
              )}
            </div>
          </div>
        ))}
      </div>
    </Card>
  );
}

function Invites({ teamId, admin }: { teamId: string; admin: boolean }) {
  const [invites, setInvites] = useState<TeamInvite[]>([]);
  const [email, setEmail] = useState("");
  const [role, setRole] = useState("member");
  const [error, setError] = useState("");

  const load = useCallback(
    () => api.invites(teamId).then((r) => setInvites(r.invites)).catch(() => {}),
    [teamId],
  );
  useEffect(() => {
    load();
  }, [load]);

  async function invite(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    try {
      await api.createInvite(teamId, email, role);
      setEmail("");
      load();
    } catch (err) {
      setError(err instanceof Error ? err.message : "failed");
    }
  }

  return (
    <Card className="p-5">
      <h2 className="mb-1 text-sm font-medium">Invites</h2>
      <p className="mb-4 text-xs text-muted-foreground">
        Emailed invite links expire after 30 days; the recipient's account email must match.
      </p>
      {admin && (
        <form onSubmit={invite} className="mb-4 grid grid-cols-[1fr_120px_auto] gap-2">
          <Input
            type="email"
            placeholder="user@example.com"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            required
          />
          <ComboBox
            value={role}
            onChange={setRole}
            options={[
              { value: "member", label: "member" },
              { value: "admin", label: "admin" },
              { value: "owner", label: "owner" },
            ]}
            placeholder="role…"
          />
          <Button type="submit" size="sm">
            Invite
          </Button>
        </form>
      )}
      <Err msg={error} />
      <div className="stagger grid gap-2">
        {invites.map((i) => (
          <div
            key={i.id}
            className="flex items-center justify-between rounded-md border border-border px-3 py-2"
          >
            <div className="text-sm">
              {i.email}
              <span className="ml-2 text-xs text-muted-foreground">
                {i.role} · expires {new Date(i.expires_at).toLocaleDateString()}
              </span>
            </div>
            <div className="flex items-center gap-2">
              <Badge variant={i.status === "pending" ? "warning" : "secondary"}>{i.status}</Badge>
              {admin && i.status === "pending" && (
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => api.revokeInvite(teamId, i.id).then(load).catch(() => {})}
                >
                  Revoke
                </Button>
              )}
            </div>
          </div>
        ))}
        {invites.length === 0 && (
          <p className="text-xs text-muted-foreground">No invites.</p>
        )}
      </div>
    </Card>
  );
}

const DB_ENGINES = ["sqlite", "postgres", "mongodb"];

function StorageSection({ teamId, admin }: { teamId: string; admin: boolean }) {
  const [items, setItems] = useState<Storage[]>([]);
  const [projects, setProjects] = useState<Project[]>([]);
  const [name, setName] = useState("");
  const [type, setType] = useState("database");
  const [engine, setEngine] = useState("postgres");
  const [linkTarget, setLinkTarget] = useState<Record<string, string>>({});
  const [error, setError] = useState("");

  const load = useCallback(() => {
    api.storage(teamId).then((r) => setItems(r.storage)).catch((e) => setError(e.message));
    api
      .projects()
      .then((r) => setProjects(r.projects.filter((p) => p.team_id === teamId)))
      .catch(() => {});
  }, [teamId]);
  useEffect(() => {
    load();
  }, [load]);

  async function create(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    try {
      await api.createStorage(teamId, {
        name,
        type,
        ...(type === "database" || type === "kv" ? { engine } : {}),
      });
      setName("");
      load();
    } catch (err) {
      setError(err instanceof Error ? err.message : "failed");
    }
  }

  const act = (p: Promise<unknown>) => p.then(load).catch((e) => setError(e.message));

  return (
    <Card className="p-5">
      <h2 className="mb-1 text-sm font-medium">Storage</h2>
      <p className="mb-4 text-xs text-muted-foreground">
        Team-scoped databases, volumes, and key-value stores. Linked storages are mounted at
        /data and reachable on a private network from deployments.
      </p>
      {admin && (
        <form onSubmit={create} className="mb-4 grid grid-cols-[1fr_120px_130px_auto] gap-2">
          <Input
            placeholder="name [a-z0-9-_]"
            value={name}
            onChange={(e) => setName(e.target.value)}
            required
          />
          <ComboBox
            value={type}
            onChange={setType}
            options={[
              { value: "database", label: "database" },
              { value: "kv", label: "kv" },
              { value: "volume", label: "volume" },
              { value: "queue", label: "queue" },
            ]}
            placeholder="type…"
          />
          {type === "database" ? (
            <ComboBox
              value={engine}
              onChange={setEngine}
              options={DB_ENGINES.map((e2) => ({ value: e2, label: e2 }))}
              placeholder="engine…"
              searchPlaceholder="Search engines…"
            />
          ) : type === "kv" ? (
            <ComboBox value="redis" onChange={() => {}} options={[{ value: "redis", label: "redis" }]} disabled />
          ) : (
            <span />
          )}
          <Button type="submit" size="sm">
            Create
          </Button>
        </form>
      )}
      <Err msg={error} />
      <div className="stagger grid gap-2">
        {items.map((s) => (
          <div key={s.id} className="rounded-md border border-border px-3 py-2">
            <div className="flex items-center justify-between">
              <div className="text-sm">
                {s.name}
                <span className="ml-2 text-xs text-muted-foreground">
                  {s.type} · {s.engine}
                </span>
              </div>
              <div className="flex items-center gap-2">
                <Badge variant={s.status === "ready" ? "success" : "warning"}>{s.status}</Badge>
                {admin && (
                  <>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => act(api.resetStorage(teamId, s.id))}
                    >
                      Reset
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => act(api.deleteStorage(teamId, s.id))}
                    >
                      Delete
                    </Button>
                  </>
                )}
              </div>
            </div>
            {s.error && <p className="mt-1 text-xs text-destructive">{s.error}</p>}
            <div className="mt-2 flex flex-wrap items-center gap-2">
              {s.links.map((l) => (
                <Badge key={l.project_id} variant="outline" className="gap-1">
                  <Link to={`/projects/${l.project_id}`} className="hover:underline">
                    {l.project_name}
                  </Link>
                  {admin && (
                    <button
                      className="ml-1 text-muted-foreground hover:text-foreground"
                      onClick={() => act(api.unlinkStorage(teamId, s.id, l.project_id))}
                    >
                      ×
                    </button>
                  )}
                </Badge>
              ))}
              {admin && projects.length > 0 && (
                <div className="flex items-center gap-1">
                  <ComboBox
                    className="w-40"
                    value={linkTarget[s.id] ?? ""}
                    onChange={(v) => setLinkTarget((t) => ({ ...t, [s.id]: v }))}
                    options={projects
                      .filter((p) => !s.links.some((l) => l.project_id === p.id))
                      .map((p) => ({ value: p.id, label: p.name }))}
                    placeholder="link project…"
                    searchPlaceholder="Search projects…"
                  />
                  {linkTarget[s.id] && (
                    <Button
                      variant="outline"
                      size="sm"
                      className="h-7"
                      onClick={() => act(api.linkStorage(teamId, s.id, linkTarget[s.id]))}
                    >
                      Link
                    </Button>
                  )}
                </div>
              )}
            </div>
          </div>
        ))}
        {items.length === 0 && <p className="text-xs text-muted-foreground">No storage yet.</p>}
      </div>
    </Card>
  );
}

function Webhooks({ teamId, admin }: { teamId: string; admin: boolean }) {
  const [hooks, setHooks] = useState<TeamWebhook[]>([]);
  const [name, setName] = useState("");
  const [url, setUrl] = useState("");
  const [secret, setSecret] = useState("");
  const [error, setError] = useState("");

  const load = useCallback(
    () => api.teamWebhooks(teamId).then((r) => setHooks(r.webhooks)).catch(() => {}),
    [teamId],
  );
  useEffect(() => {
    load();
  }, [load]);

  async function create(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    try {
      await api.createTeamWebhook(teamId, {
        name,
        url,
        ...(secret ? { secret } : {}),
      });
      setName("");
      setUrl("");
      setSecret("");
      load();
    } catch (err) {
      setError(err instanceof Error ? err.message : "failed");
    }
  }

  return (
    <Card className="p-5">
      <h2 className="mb-1 text-sm font-medium">Team webhooks</h2>
      <p className="mb-4 text-xs text-muted-foreground">
        Deployment events for every project in this team, signed with X-Runway-Signature.
      </p>
      {admin && (
        <form onSubmit={create} className="mb-4 grid grid-cols-[1fr_2fr_1fr_auto] gap-2">
          <Input placeholder="name" value={name} onChange={(e) => setName(e.target.value)} required />
          <Input
            type="url"
            placeholder="https://example.com/hook"
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            required
          />
          <Input
            type="password"
            placeholder="secret (optional)"
            value={secret}
            onChange={(e) => setSecret(e.target.value)}
          />
          <Button type="submit" size="sm">
            Add
          </Button>
        </form>
      )}
      <Err msg={error} />
      <div className="stagger grid gap-2">
        {hooks.map((h) => (
          <div
            key={h.id}
            className="flex items-center justify-between rounded-md border border-border px-3 py-2"
          >
            <div className="text-sm">
              {h.name}
              <span className="ml-2 font-mono text-xs text-muted-foreground">{h.url}</span>
            </div>
            <div className="flex items-center gap-2">
              <Badge variant={h.status === "active" ? "success" : "secondary"}>{h.status}</Badge>
              {admin && (
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => api.deleteTeamWebhook(teamId, h.id).then(load).catch(() => {})}
                >
                  Delete
                </Button>
              )}
            </div>
          </div>
        ))}
        {hooks.length === 0 && <p className="text-xs text-muted-foreground">No webhooks.</p>}
      </div>
    </Card>
  );
}

function Cloudflare({ teamId, admin }: { teamId: string; admin: boolean }) {
  const [status, setStatus] = useState<Awaited<ReturnType<typeof api.cfStatus>> | null>(null);
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const load = useCallback(
    () => api.cfStatus(teamId).then(setStatus).catch((e) => setError(e.message)),
    [teamId],
  );
  useEffect(() => {
    load();
  }, [load]);

  async function connect(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    setBusy(true);
    try {
      await api.cfConnect(teamId, token.trim());
      setToken("");
      await load();
    } catch (err) {
      setError(err instanceof Error ? err.message : "connect failed");
    } finally {
      setBusy(false);
    }
  }

  return (
    <Card className="p-5">
      <h2 className="mb-1 text-sm font-medium">Cloudflare</h2>
      <p className="mb-4 text-xs text-muted-foreground">
        One-token setup: DNS records are created automatically when you assign a domain. When
        the instance sits behind CGNAT or a private IP, a cloudflared tunnel is created and
        hostnames route through it — no inbound ports needed.
      </p>
      {status?.connected ? (
        <div className="flex items-center justify-between">
          <div className="text-sm">
            <span className="text-muted-foreground">Account:</span> {status.account_name}
            {status.tunnel_name && (
              <span className="ml-3 text-xs text-muted-foreground">
                tunnel {status.tunnel_name} · {status.container_status ?? "unknown"}
              </span>
            )}
          </div>
          <div className="flex items-center gap-2">
            <Badge variant="success">connected</Badge>
            {admin && (
              <Button
                variant="ghost"
                size="sm"
                onClick={() => api.cfDisconnect(teamId).then(load).catch((e) => setError(e.message))}
              >
                Disconnect
              </Button>
            )}
          </div>
        </div>
      ) : (
        admin && (
          <form onSubmit={connect} className="flex gap-2">
            <Input
              type="password"
              placeholder="API token (Zone.DNS edit on your zones)"
              value={token}
              onChange={(e) => setToken(e.target.value)}
              required
            />
            <Button type="submit" size="sm" disabled={busy}>
              {busy ? "Connecting…" : "Connect"}
            </Button>
          </form>
        )
      )}
      {!status?.connected && !admin && (
        <p className="text-xs text-muted-foreground">Not connected.</p>
      )}
      <Err msg={error} />
    </Card>
  );
}

function Audit({ teamId }: { teamId: string }) {
  const [entries, setEntries] = useState<AuditEntry[]>([]);
  const [error, setError] = useState("");
  useEffect(() => {
    api.teamAudit(teamId).then((r) => setEntries(r.entries)).catch((e) => setError(e.message));
  }, [teamId]);
  return (
    <Card className="p-5">
      <h2 className="mb-4 text-sm font-medium">Audit log</h2>
      <Err msg={error} />
      <div className="grid gap-1 font-mono text-xs">
        {entries.map((e) => (
          <div key={e.id} className="flex items-baseline justify-between gap-4">
            <span className="text-muted-foreground">
              {new Date(e.created_at).toLocaleString()}
            </span>
            <span className="flex-1 truncate">
              {e.action}
              {e.username ? ` by ${e.username}` : ""}
              {e.detail ? ` — ${e.detail}` : ""}
            </span>
          </div>
        ))}
        {entries.length === 0 && !error && (
          <p className="text-muted-foreground">No entries.</p>
        )}
      </div>
    </Card>
  );
}

export default function TeamPage({ me }: { me: Me }) {
  const { id } = useParams<{ id: string }>();
  const nav = useNavigate();
  const [team, setTeam] = useState<TeamDetail | null>(null);
  const [name, setName] = useState("");
  const [error, setError] = useState("");

  const load = useCallback(() => {
    if (!id) return;
    api.team(id).then((r) => {
      setTeam(r.team);
      setName(r.team.name);
    }).catch((e) => setError(e.message));
  }, [id]);
  useEffect(() => {
    load();
  }, [load]);

  if (!team) {
    return <div className="page-enter mx-auto max-w-5xl p-4 sm:p-8">{error || "Loading…"}</div>;
  }

  const myRole = team.members.find((m) => m.user_id === me.id)?.role ?? "member";
  const admin = myRole === "owner" || myRole === "admin";

  async function rename(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    try {
      await api.renameTeam(team!.id, name);
      load();
    } catch (err) {
      setError(err instanceof Error ? err.message : "failed");
    }
  }

  return (
    <div className="page-enter mx-auto max-w-5xl p-4 sm:p-8">
      <div className="mb-6 flex items-center justify-between">
        <div className="flex items-center gap-3">
          <Avatar kind="team" id={team.id} name={team.name} hasAvatar={team.has_avatar} className="h-9 w-9 text-sm" />
          <div>
            <h1 className="text-xl font-semibold">{team.name}</h1>
            <p className="text-xs text-muted-foreground">{team.slug}</p>
          </div>
        </div>
        <div className="flex items-center gap-2">
          {admin && (
            <form onSubmit={rename} className="flex gap-2">
              <Input value={name} onChange={(e) => setName(e.target.value)} required />
              <Button type="submit" variant="outline" size="sm">
                Rename
              </Button>
            </form>
          )}
          {myRole === "owner" && (
            <Button
              variant="destructive"
              size="sm"
              onClick={() =>
                api
                  .deleteTeam(team.id)
                  .then(() => nav("/teams"))
                  .catch((e) => setError(e.message))
              }
            >
              Delete team
            </Button>
          )}
        </div>
      </div>
      <Err msg={error} />
      <div className="grid gap-4">
        {admin && (
          <Card className="p-5">
            <h2 className="mb-3 text-sm font-medium">Avatar</h2>
            <AvatarRow
              kind="team"
              id={team.id}
              name={team.name}
              hasAvatar={team.has_avatar}
              onChanged={load}
            />
          </Card>
        )}
        <Members team={team} myRole={myRole} me={me} reload={load} />
        <Invites teamId={team.id} admin={admin} />
        <StorageSection teamId={team.id} admin={admin} />
        <Cloudflare teamId={team.id} admin={admin} />
        <Webhooks teamId={team.id} admin={admin} />
        {admin && <Audit teamId={team.id} />}
      </div>
    </div>
  );
}
