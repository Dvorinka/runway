import {
  api,
  type AllowlistRule,
  type GitConnection,
  type Me,
  type NodeTlsProvision,
  type RemoteNode,
} from "@/lib/api";
import { AvatarRow, Badge, Button, Card, ComboBox, Input } from "@/components/ui";
import { useEffect, useState } from "react";

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
        <ComboBox
          value={provider}
          onChange={setProvider}
          options={[
            { value: "gitea", label: "Gitea" },
            { value: "gitlab", label: "GitLab" },
            { value: "bitbucket", label: "Bitbucket" },
          ]}
          placeholder="Provider…"
        />
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
          placeholder={
            provider === "bitbucket" ? "OAuth key:secret or access token" : "access token"
          }
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

function GithubApp() {
  const [st, setSt] = useState<Awaited<ReturnType<typeof api.githubAppStatus>> | null>(null);

  useEffect(() => {
    api.githubAppStatus().then(setSt).catch(() => {});
  }, []);

  return (
    <Card className="p-5">
      <h2 className="mb-1 text-sm font-medium">GitHub App</h2>
      <p className="mb-4 text-xs text-muted-foreground">
        Powers repository browsing, push-triggered deploys, and preview URLs for GitHub projects.
      </p>
      {st === null ? (
        <p className="text-xs text-muted-foreground">Loading…</p>
      ) : st.configured ? (
        <div className="grid gap-2">
          <div className="flex items-center gap-2">
            <Badge variant="success">configured</Badge>
            <span className="text-xs text-muted-foreground">
              via {st.source === "env" ? "environment" : "instance registration"}
            </span>
          </div>
          {st.slug && <p className="font-mono text-xs">@{st.slug}</p>}
          {st.install_url && (
            <a
              className="w-fit text-xs text-muted-foreground underline-offset-2 hover:underline"
              href={st.install_url}
              target="_blank"
              rel="noreferrer"
            >
              Install / manage the app on GitHub →
            </a>
          )}
        </div>
      ) : (
        <div className="grid gap-2">
          <a href="/api/v1/github/app/register" className="w-fit">
            <Button size="sm">Register with GitHub</Button>
          </a>
          <p className="text-xs text-muted-foreground">
            Creates a private GitHub App on your account with the permissions Runway needs —
            credentials are stored encrypted on this instance. The GITHUB_APP_* environment
            variables remain a manual alternative.
          </p>
        </div>
      )}
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
        <ComboBox
          value={type}
          onChange={setType}
          options={[
            { value: "email", label: "email" },
            { value: "domain", label: "domain" },
            { value: "pattern", label: "pattern" },
          ]}
          placeholder="type…"
        />
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

function JobQueue() {
  const [data, setData] = useState<Awaited<ReturnType<typeof api.adminJobs>> | null>(null);
  const [filter, setFilter] = useState("");
  const [error, setError] = useState("");

  const load = (status = filter) =>
    api.adminJobs(status || undefined).then(setData).catch((e) => setError(e.message));
  useEffect(() => {
    load();
    const t = setInterval(load, 5000);
    return () => clearInterval(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [filter]);

  const jobs = data?.jobs ?? [];
  return (
    <Card className="p-5">
      <h2 className="mb-1 text-sm font-medium">Job queue</h2>
      <p className="mb-4 text-xs text-muted-foreground">
        Background work — deploys, cleanups, tunnel syncs. Failed jobs can be retried.
      </p>
      <div className="mb-3 flex flex-wrap gap-1.5">
        <Badge
          variant={filter === "" ? "default" : "outline"}
          className="cursor-pointer"
          onClick={() => setFilter("")}
        >
          all
        </Badge>
        {(data?.counts ?? []).map((c) => (
          <Badge
            key={c.status}
            variant={filter === c.status ? "default" : "outline"}
            className="cursor-pointer"
            onClick={() => setFilter(c.status === filter ? "" : c.status)}
          >
            {c.status} {c.count}
          </Badge>
        ))}
      </div>
      {error && <p className="mb-3 text-xs text-destructive">{error}</p>}
      {jobs.length === 0 ? (
        <p className="text-xs text-muted-foreground">Queue is empty.</p>
      ) : (
        <div className="grid gap-1.5">
          {jobs.map((j) => (
            <div
              key={j.id}
              className="flex items-center justify-between gap-3 rounded-md border border-border px-3 py-2 text-xs"
            >
              <div className="min-w-0">
                <span className="font-mono">#{j.id}</span>{" "}
                <span className="font-medium">{j.kind}</span>{" "}
                <Badge
                  variant={
                    j.status === "failed"
                      ? "destructive"
                      : j.status === "running"
                        ? "default"
                        : "outline"
                  }
                  className="ml-1"
                >
                  {j.status}
                </Badge>
                {j.last_error && (
                  <p className="mt-1 truncate text-muted-foreground">{j.last_error}</p>
                )}
              </div>
              <div className="flex shrink-0 items-center gap-2 text-muted-foreground">
                <span title={`attempts: ${j.attempts}`}>×{j.attempts}</span>
                {j.status === "failed" && (
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => api.retryJob(j.id).then(() => load()).catch(() => {})}
                  >
                    Retry
                  </Button>
                )}
              </div>
            </div>
          ))}
        </div>
      )}
    </Card>
  );
}

function Nodes() {
  const [nodes, setNodes] = useState<RemoteNode[]>([]);
  const [name, setName] = useState("");
  const [host, setHost] = useState("");
  const [url, setUrl] = useState("");
  const [error, setError] = useState("");
  const [bundle, setBundle] = useState<NodeTlsProvision | null>(null);

  const load = () => api.nodes().then((r) => setNodes(r.nodes)).catch((e) => setError(e.message));
  useEffect(() => {
    load();
  }, []);

  async function provision(id: string) {
    setError("");
    try {
      setBundle(await api.provisionNodeTls(id));
      load();
    } catch (err) {
      setError(err instanceof Error ? err.message : "failed");
    }
  }

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
              {n.tls && <Badge variant="success">mTLS</Badge>}
              <Badge variant={n.status === "online" ? "success" : "warning"}>{n.status}</Badge>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => api.checkNode(n.id).then(load).catch(() => {})}
              >
                Check
              </Button>
              {n.tls ? (
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => api.clearNodeTls(n.id).then(load).catch(() => {})}
                >
                  Clear TLS
                </Button>
              ) : (
                <Button variant="ghost" size="sm" onClick={() => provision(n.id)}>
                  mTLS
                </Button>
              )}
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
      {bundle && (
        <div className="mt-4 rounded-md border border-border bg-muted/30 p-3">
          <div className="mb-2 flex items-center justify-between">
            <p className="text-xs font-medium">
              Server material for {bundle.node.name} — shown once
            </p>
            <Button variant="ghost" size="sm" onClick={() => setBundle(null)}>
              Dismiss
            </Button>
          </div>
          <p className="mb-2 text-xs text-muted-foreground">
            Install on the remote host, restart dockerd with the flags below, then run Check.
          </p>
          <pre className="max-h-64 overflow-auto rounded bg-background p-3 font-mono text-[11px] whitespace-pre-wrap">
            {`# /etc/docker/runway/ca.pem\n${bundle.ca_pem}\n# /etc/docker/runway/server.pem\n${bundle.server_cert_pem}\n# /etc/docker/runway/server-key.pem\n${bundle.server_key_pem}\n# dockerd flags\n--tlsverify ${Object.entries(bundle.dockerd)
              .filter(([k]) => k !== "tlsverify")
              .map(([k, v]) => `--${k}=${v}`)
              .join(" ")}`}
          </pre>
        </div>
      )}
    </Card>
  );
}

function Account({ me }: { me: Me }) {
  const [name, setName] = useState(me.name ?? "");
  const [username, setUsername] = useState(me.username);
  const [email, setEmail] = useState(me.email);
  const [profileMsg, setProfileMsg] = useState("");
  const [profileErr, setProfileErr] = useState("");
  const [cur, setCur] = useState("");
  const [next, setNext] = useState("");
  const [pwMsg, setPwMsg] = useState("");
  const [pwErr, setPwErr] = useState("");
  const [delPw, setDelPw] = useState("");
  const [delErr, setDelErr] = useState("");
  const [confirming, setConfirming] = useState(false);

  async function saveProfile(e: React.FormEvent) {
    e.preventDefault();
    setProfileMsg("");
    setProfileErr("");
    try {
      await api.updateMe({ name, username, email });
      setProfileMsg("saved");
    } catch (err) {
      setProfileErr(err instanceof Error ? err.message : "failed");
    }
  }

  async function savePassword(e: React.FormEvent) {
    e.preventDefault();
    setPwMsg("");
    setPwErr("");
    try {
      await api.changePassword(cur, next);
      setPwMsg("password changed");
      setCur("");
      setNext("");
    } catch (err) {
      setPwErr(err instanceof Error ? err.message : "failed");
    }
  }

  async function deleteAccount(e: React.FormEvent) {
    e.preventDefault();
    setDelErr("");
    try {
      await api.deleteMe(delPw);
      location.href = "/login";
    } catch (err) {
      setDelErr(err instanceof Error ? err.message : "failed");
    }
  }

  return (
    <>
      <Card className="p-5">
        <h2 className="mb-1 text-sm font-medium">Account</h2>
        <p className="mb-4 text-xs text-muted-foreground">Profile and sign-in details.</p>
        <AvatarRow
          kind="user"
          id={me.id}
          name={me.name ?? me.username}
          hasAvatar={me.has_avatar}
          onChanged={() => location.reload()}
        />
        <form onSubmit={saveProfile} className="grid gap-3 sm:grid-cols-3">
          <Input
            placeholder="Display name"
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
          <Input
            placeholder="username"
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            required
          />
          <div className="relative">
            <Input
              type="email"
              placeholder="email"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              required
              className="pr-16"
            />
            {me.email_verified === false && me.email === email && (
              <button
                type="button"
                onClick={async () => {
                  try {
                    await api.emailResend();
                    setProfileMsg("verification email sent");
                  } catch (err) {
                    setProfileErr(err instanceof Error ? err.message : "failed");
                  }
                }}
                className="absolute right-3 top-1/2 -translate-y-1/2 text-xs text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
              >
                verify
              </button>
            )}
          </div>
          <div className="flex items-center gap-3 sm:col-span-3">
            <Button type="submit" size="sm">
              Save
            </Button>
            {profileMsg && <span className="text-xs text-emerald-500">{profileMsg}</span>}
            {profileErr && <span className="text-xs text-destructive">{profileErr}</span>}
          </div>
        </form>
      </Card>
      <Card className="p-5">
        <h2 className="mb-1 text-sm font-medium">Change password</h2>
        <form onSubmit={savePassword} className="mt-3 grid gap-3 sm:grid-cols-3">
          <Input
            type="password"
            placeholder="current password"
            value={cur}
            onChange={(e) => setCur(e.target.value)}
            required
          />
          <Input
            type="password"
            placeholder="new password"
            value={next}
            onChange={(e) => setNext(e.target.value)}
            required
          />
          <div className="flex items-center gap-3">
            <Button type="submit" size="sm">
              Change
            </Button>
            {pwMsg && <span className="text-xs text-emerald-500">{pwMsg}</span>}
            {pwErr && <span className="text-xs text-destructive">{pwErr}</span>}
          </div>
        </form>
      </Card>
      <TwoFactor enabled={!!me.totp_enabled} />
      <Card className="border-destructive/40 p-5">
        <h2 className="mb-1 text-sm font-medium text-destructive">Delete account</h2>
        <p className="mb-4 text-xs text-muted-foreground">
          Marks the account deleted and signs you out. Projects and teams are retained.
        </p>
        {confirming ? (
          <form onSubmit={deleteAccount} className="flex items-center gap-2">
            <Input
              type="password"
              placeholder="confirm with your password"
              value={delPw}
              onChange={(e) => setDelPw(e.target.value)}
              required
              className="max-w-xs"
            />
            <Button type="submit" variant="destructive" size="sm">
              Delete
            </Button>
            <Button type="button" variant="ghost" size="sm" onClick={() => setConfirming(false)}>
              Cancel
            </Button>
            {delErr && <span className="text-xs text-destructive">{delErr}</span>}
          </form>
        ) : (
          <Button variant="outline" size="sm" onClick={() => setConfirming(true)}>
            Delete account…
          </Button>
        )}
      </Card>
    </>
  );
}

function TwoFactor({ enabled: initiallyEnabled }: { enabled: boolean }) {
  const [enabled, setEnabled] = useState(initiallyEnabled);
  const [phase, setPhase] = useState<"idle" | "enroll" | "codes" | "disable">("idle");
  const [secret, setSecret] = useState("");
  const [qrUrl, setQrUrl] = useState("");
  const [code, setCode] = useState("");
  const [codes, setCodes] = useState<string[]>([]);
  const [password, setPassword] = useState("");
  const [error, setError] = useState("");

  async function enroll() {
    setError("");
    try {
      const res = await api.totpEnroll();
      setSecret(res.secret);
      const qrcode = await import("qrcode");
      setQrUrl(await qrcode.toDataURL(res.otpauth_url, { margin: 1, width: 180 }));
      setPhase("enroll");
    } catch (e) {
      setError(e instanceof Error ? e.message : "enroll failed");
    }
  }

  async function verify(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    try {
      const res = await api.totpVerify(code);
      setCodes(res.recovery_codes);
      setCode("");
      setPhase("codes");
      setEnabled(true);
    } catch (e) {
      setError(e instanceof Error ? e.message : "invalid code");
    }
  }

  async function disable(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    try {
      await api.totpDisable(password);
      setPassword("");
      setPhase("idle");
      setEnabled(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : "invalid password");
    }
  }

  return (
    <Card className="p-5">
      <div className="mb-1 flex items-center justify-between">
        <h2 className="text-sm font-medium">Two-factor authentication</h2>
        {enabled && <Badge variant="success">on</Badge>}
      </div>
      <p className="mb-4 text-xs text-muted-foreground">
        TOTP via any authenticator app — sign-in asks for a 6-digit code after
        your password. Recovery codes are shown once at setup.
      </p>

      {phase === "idle" &&
        (enabled ? (
          <Button variant="outline" size="sm" onClick={() => setPhase("disable")}>
            Disable…
          </Button>
        ) : (
          <Button size="sm" onClick={enroll}>
            Set up two-factor
          </Button>
        ))}

      {phase === "enroll" && (
        <form onSubmit={verify} className="grid gap-3">
          <div className="flex items-start gap-4">
            {qrUrl && (
              <img src={qrUrl} alt="TOTP QR code" className="rounded-md border border-border" />
            )}
            <div className="text-xs text-muted-foreground">
              <p className="mb-2">Scan with your authenticator app, or enter manually:</p>
              <code className="block break-all rounded-md border border-border bg-black/40 p-2 font-mono">
                {secret}
              </code>
            </div>
          </div>
          <div className="flex items-center gap-2">
            <Input
              placeholder="6-digit code"
              value={code}
              onChange={(e) => setCode(e.target.value.replace(/\D/g, "").slice(0, 6))}
              required
              className="w-36 font-mono"
            />
            <Button type="submit" size="sm">
              Enable
            </Button>
            <Button type="button" variant="ghost" size="sm" onClick={() => setPhase("idle")}>
              Cancel
            </Button>
          </div>
        </form>
      )}

      {phase === "codes" && (
        <div className="grid gap-3">
          <p className="text-xs text-amber-400">
            Save these recovery codes — each works once in place of an
            authenticator code. They are shown only now.
          </p>
          <div className="grid grid-cols-2 gap-1 rounded-md border border-border bg-black/40 p-3 font-mono text-xs sm:grid-cols-4">
            {codes.map((c) => (
              <span key={c}>{c}</span>
            ))}
          </div>
          <div>
            <Button size="sm" onClick={() => setPhase("idle")}>
              Done
            </Button>
          </div>
        </div>
      )}

      {phase === "disable" && (
        <form onSubmit={disable} className="flex items-center gap-2">
          <Input
            type="password"
            placeholder="confirm with your password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            required
            className="max-w-xs"
          />
          <Button type="submit" variant="destructive" size="sm">
            Disable 2FA
          </Button>
          <Button type="button" variant="ghost" size="sm" onClick={() => setPhase("idle")}>
            Cancel
          </Button>
        </form>
      )}
      {error && <p className="mt-2 text-xs text-destructive">{error}</p>}
    </Card>
  );
}

export default function SettingsPage({ me }: { me: Me }) {
  return (
    <div className="page-enter mx-auto max-w-5xl p-4 sm:p-8">
      <h1 className="text-xl font-semibold">Settings</h1>
      <p className="mb-6 mt-1 text-sm text-muted-foreground">
        Your account, providers, access control, and instance infrastructure.
      </p>
      <div className="grid gap-4">
        <Account me={me} />
        <GitProviders />
        {me.id === 1 && (
          <>
            <GithubApp />
            <Allowlist />
            <JobQueue />
            <Nodes />
          </>
        )}
      </div>
    </div>
  );
}
