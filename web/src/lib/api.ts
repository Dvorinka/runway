export interface Project {
  id: string;
  team_id: string;
  name: string;
  slug: string | null;
  description: string | null;
  url: string | null;
  repo_provider: string;
  repo_full_name: string;
  repo_branch: string;
  remote_node_id: string | null;
  has_avatar?: boolean;
  config: Record<string, unknown>;
  environments?: { id: string; name: string; slug: string; branch?: string; status?: string }[];
  status: string;
  preset?: string | null;
  latest_deployment?: Deployment | null;
  created_at: string;
}

export interface Deployment {
  id: string;
  project_id: string;
  project_name?: string;
  environment_id: string;
  branch: string;
  commit_sha: string;
  commit_meta?: { message?: string; author?: string; author_name?: string };
  status: string;
  conclusion: string | null;
  computed_status?: string | null;
  container_status: string | null;
  trigger?: string;
  error?: string | { message?: string; status?: string } | null;
  url?: string;
  urls?: { immutable?: string | null; environment?: string | null; branch?: string | null };
  concluded_at?: string | null;
  created_at: string;
}

export interface Me {
  id: number;
  email: string;
  username: string;
  name: string | null;
  has_avatar: boolean;
}

export interface EnvVar {
  key: string;
  value: string;
  environment: string | null;
}

export interface CronJob {
  id: string;
  name: string;
  schedule: string;
  branch: string;
  environment_id: string | null;
  enabled: boolean;
  last_run_at: string | null;
  next_run_at: string | null;
}

export interface RedirectRule {
  id: string;
  source_path: string;
  target_url: string;
  status_code: number;
  enabled: boolean;
}

export interface Webhook {
  id: string;
  name: string;
  url: string;
  has_secret: boolean;
  events: string[];
  status: string;
}

export interface Domain {
  id: string;
  hostname: string;
  type: string;
  status: string;
}

export interface GitConnection {
  id: number;
  base_url: string | null;
  username: string | null;
}

export interface Repo {
  id: number | string;
  full_name: string;
  name: string;
  default_branch: string;
}

export interface AllowlistRule {
  id: number;
  type: string;
  value: string;
}

export interface RemoteNode {
  id: string;
  name: string;
  host: string;
  docker_url: string;
  status: string;
  max_deployments: number | null;
}

export interface Team {
  id: string;
  name: string;
  slug: string | null;
  role: string;
  has_avatar?: boolean;
}

export interface TeamMember {
  user_id: number;
  username: string;
  role: string;
}

export interface TeamDetail {
  id: string;
  name: string;
  slug: string | null;
  has_avatar?: boolean;
  members: TeamMember[];
}

export interface TeamInvite {
  id: string;
  email: string;
  role: string;
  status: string;
  expires_at: string;
}

export interface AuditEntry {
  id: number;
  action: string;
  username: string | null;
  user_id: number | null;
  project_id: string | null;
  resource_type: string | null;
  resource_id: string | null;
  detail: string | null;
  created_at: string;
}

export interface Storage {
  id: string;
  name: string;
  type: string;
  status: string;
  engine: string;
  config: Record<string, unknown>;
  error: string | null;
  links: { project_id: string; project_name: string }[];
  created_at: string;
}

export interface AppNotification {
  id: string;
  type: string;
  title: string;
  body: string | null;
  link: string | null;
  read: boolean;
  created_at: string;
  team_id: string | null;
  project_id: string | null;
}

export interface TeamWebhook {
  id: string;
  team_id: string;
  name: string;
  url: string;
  has_secret: boolean;
  events: string[];
  project_ids: string[] | null;
  status: string;
}

export interface CreateProjectInput {
  name: string;
  provider?: string;
  repo_id?: number;
  repo_full_name: string;
  installation_id?: number;
  connection_id?: number;
  branch?: string;
  preset?: string;
}

async function req<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(path, {
    credentials: "include",
    headers: { "content-type": "application/json" },
    ...init,
  });
  if (!res.ok) {
    const body = await res.json().catch(() => ({}));
    throw new Error((body as { error?: string }).error || `${res.status}`);
  }
  if (res.status === 204) return undefined as T;
  return res.json() as Promise<T>;
}

export const api = {
  me: () => req<Me>("/api/auth/me"),
  login: (email: string, password: string) =>
    req<{ ok: boolean }>("/api/auth/login", {
      method: "POST",
      body: JSON.stringify({ email, password }),
    }),
  register: (email: string, password: string, username?: string) =>
    req<{ ok: boolean }>("/api/auth/register", {
      method: "POST",
      body: JSON.stringify({ email, password, username }),
    }),
  logout: () => req<void>("/api/auth/logout", { method: "POST" }),
  oidcInfo: () =>
    req<{ enabled: boolean; display_name: string | null }>("/api/auth/oidc/info"),
  githubAppStatus: () =>
    req<{
      configured: boolean;
      source: "env" | "db";
      slug: string | null;
      install_url: string | null;
      web_base: string;
    }>("/api/v1/github/app/status"),

  projects: () => req<{ projects: Project[] }>("/api/v1/projects"),
  project: (id: string) => req<Project>(`/api/v1/projects/${id}`),
  createProject: (body: CreateProjectInput) =>
    req<Project>("/api/v1/projects", { method: "POST", body: JSON.stringify(body) }),
  patchProject: (id: string, body: Record<string, unknown>) =>
    req<Project>(`/api/v1/projects/${id}`, { method: "PATCH", body: JSON.stringify(body) }),
  exportProject: (id: string) => req<Record<string, unknown>>(`/api/v1/projects/${id}/export`),
  importProject: (id: string, body: unknown) =>
    req<{ ok: boolean }>(`/api/v1/projects/${id}/import`, {
      method: "POST",
      body: JSON.stringify(body),
    }),

  speedInsights: (id: string, days = 7) =>
    req<{
      days: number;
      views: number;
      p75: { lcp?: number; fcp?: number; inp?: number; cls?: number; ttfb?: number };
      paths: { path: string; views: number; lcp?: number }[];
      series: { day: string; views: number; lcp?: number }[];
    }>(`/api/v1/projects/${id}/speed?days=${days}`),

  getEnv: (id: string) => req<{ env: EnvVar[] }>(`/api/v1/projects/${id}/env`),
  patchEnv: (id: string, vars: { key: string; value?: string; delete?: boolean }[]) =>
    req<{ ok: boolean }>(`/api/v1/projects/${id}/env`, {
      method: "PATCH",
      body: JSON.stringify(vars),
    }),

  deployments: (projectId: string) =>
    req<{ deployments: Deployment[] }>(`/api/v1/projects/${projectId}/deployments`),
  deploymentsIndex: () => req<{ deployments: Deployment[] }>("/api/v1/deployments"),
  deploy: (projectId: string, body: { branch?: string } = {}) =>
    req<Deployment>(`/api/v1/projects/${projectId}/deployments`, {
      method: "POST",
      body: JSON.stringify(body),
    }),
  deployment: (id: string) => req<Deployment>(`/api/v1/deployments/${id}`),
  updateMe: (body: { name?: string; username?: string; email?: string }) =>
    req<Me>("/api/auth/me", { method: "PATCH", body: JSON.stringify(body) }),
  changePassword: (current_password: string, new_password: string) =>
    req<{ ok: boolean }>("/api/auth/password", {
      method: "POST",
      body: JSON.stringify({ current_password, new_password }),
    }),
  deleteMe: (password: string) =>
    req<void>("/api/auth/me", { method: "DELETE", body: JSON.stringify({ password }) }),
  deploymentStats: (id: string) =>
    req<{
      running: boolean;
      cpu_pct?: number;
      mem_used?: number;
      mem_limit?: number;
      net_rx?: number;
      net_tx?: number;
      pids?: number;
    }>(`/api/v1/deployments/${id}/stats`),
  deploymentMetrics: (id: string) =>
    req<{
      samples: {
        ts: string;
        cpu_pct: number;
        mem_used: number;
        net_rx: number;
        net_tx: number;
        pids: number;
      }[];
    }>(`/api/v1/deployments/${id}/metrics`),
  projectLogs: (projectId: string, tail = 300) =>
    fetch(`/api/v1/projects/${projectId}/logs?tail=${tail}`, {
      credentials: "include",
    }).then((r) => r.text()),
  deploymentLogs: (id: string, tail = 500) =>
    fetch(`/api/v1/deployments/${id}/logs?tail=${tail}`, {
      credentials: "include",
    }).then((r) => r.text()),
  redeploy: (id: string) =>
    req<Deployment>(`/api/v1/deployments/${id}/redeploy`, { method: "POST" }),
  uploadDeploy: (projectId: string, body: Blob) =>
    fetch(`/api/v1/projects/${projectId}/deployments/upload`, {
      method: "POST",
      credentials: "include",
      headers: { "content-type": "application/gzip" },
      body,
    }).then(async (r) => {
      const d = await r.json().catch(() => ({}));
      if (!r.ok) throw new Error(d.error ?? `upload failed (${r.status})`);
      return d as Deployment;
    }),
  cancel: (id: string) =>
    req<Deployment>(`/api/v1/deployments/${id}/cancel`, { method: "POST" }),
  setAvatar: (file: Blob) =>
    fetch(`/api/auth/avatar`, {
      method: "PUT",
      credentials: "include",
      headers: { "content-type": file.type },
      body: file,
    }).then(async (r) => {
      if (!r.ok) throw new Error((await r.json().catch(() => ({}))).error ?? "upload failed");
    }),
  deleteAvatar: () =>
    req<void>(`/api/auth/avatar`, { method: "DELETE" }),
  setEntityAvatar: (kind: "team" | "project", id: string, file: Blob) =>
    fetch(`/api/v1/${kind === "team" ? "teams" : "projects"}/${id}/avatar`, {
      method: "PUT",
      credentials: "include",
      headers: { "content-type": file.type },
      body: file,
    }).then(async (r) => {
      if (!r.ok) throw new Error((await r.json().catch(() => ({}))).error ?? "upload failed");
    }),
  deleteEntityAvatar: (kind: "team" | "project", id: string) =>
    req<void>(`/api/v1/${kind === "team" ? "teams" : "projects"}/${id}/avatar`, {
      method: "DELETE",
    }),

  cron: (projectId: string) =>
    req<{ cron_jobs: CronJob[] }>(`/api/v1/projects/${projectId}/cron`),
  createCron: (projectId: string, body: { name: string; schedule: string; branch?: string }) =>
    req<CronJob>(`/api/v1/projects/${projectId}/cron`, {
      method: "POST",
      body: JSON.stringify(body),
    }),
  patchCron: (projectId: string, jobId: string, enabled: boolean) =>
    req<CronJob>(`/api/v1/projects/${projectId}/cron/${jobId}`, {
      method: "PATCH",
      body: JSON.stringify({ enabled }),
    }),
  deleteCron: (projectId: string, jobId: string) =>
    req<void>(`/api/v1/projects/${projectId}/cron/${jobId}`, { method: "DELETE" }),

  redirects: (projectId: string) =>
    req<{ redirect_rules: RedirectRule[] }>(`/api/v1/projects/${projectId}/redirects`),
  createRedirect: (
    projectId: string,
    body: { source_path: string; target_url: string; status_code?: number },
  ) =>
    req<RedirectRule>(`/api/v1/projects/${projectId}/redirects`, {
      method: "POST",
      body: JSON.stringify(body),
    }),
  patchRedirect: (projectId: string, rid: string, body: Partial<RedirectRule>) =>
    req<RedirectRule>(`/api/v1/projects/${projectId}/redirects/${rid}`, {
      method: "PATCH",
      body: JSON.stringify(body),
    }),
  deleteRedirect: (projectId: string, rid: string) =>
    req<void>(`/api/v1/projects/${projectId}/redirects/${rid}`, { method: "DELETE" }),

  webhooks: (projectId: string) =>
    req<{ webhooks: Webhook[] }>(`/api/v1/projects/${projectId}/webhooks`),
  createWebhook: (
    projectId: string,
    body: { name: string; url: string; secret?: string; events?: string[] },
  ) =>
    req<Webhook>(`/api/v1/projects/${projectId}/webhooks`, {
      method: "POST",
      body: JSON.stringify(body),
    }),
  deleteWebhook: (projectId: string, webhookId: string) =>
    req<void>(`/api/v1/projects/${projectId}/webhooks/${webhookId}`, { method: "DELETE" }),

  domains: (projectId: string) =>
    req<{ domains: Domain[] }>(`/api/v1/projects/${projectId}/domains`),
  addDomain: (projectId: string, hostname: string) =>
    req<Domain>(`/api/v1/projects/${projectId}/domains`, {
      method: "POST",
      body: JSON.stringify({ hostname, type: "route" }),
    }),
  deleteDomain: (projectId: string, domainId: string) =>
    req<void>(`/api/v1/projects/${projectId}/domains/${domainId}`, { method: "DELETE" }),
  verifyDomain: (projectId: string, domainId: string) =>
    req<{ ok: boolean; status?: string }>(
      `/api/v1/projects/${projectId}/domains/${domainId}/verify`,
      { method: "POST", body: "{}" },
    ),
  assignCloudflareDomain: (projectId: string, domainId: string) =>
    req<{ ok: boolean }>(
      `/api/v1/projects/${projectId}/domains/${domainId}/assign-cloudflare`,
      { method: "POST", body: "{}" },
    ),

  cfStatus: (teamId: string) =>
    req<{
      connected: boolean;
      account_name?: string;
      auth_method?: string;
      tunnel_name?: string;
      container_status?: string;
    }>(`/api/v1/teams/${teamId}/cloudflare`),
  cfConnect: (teamId: string, apiToken: string) =>
    req<void>(`/api/v1/teams/${teamId}/cloudflare/connect`, {
      method: "POST",
      body: JSON.stringify({ api_token: apiToken }),
    }),
  cfDisconnect: (teamId: string) =>
    req<void>(`/api/v1/teams/${teamId}/cloudflare`, { method: "DELETE" }),

  gitConnect: (provider: string, body: { base_url?: string; workspace?: string; token: string }) =>
    req<GitConnection>(`/api/v1/git/${provider}/connect`, {
      method: "POST",
      body: JSON.stringify(body),
    }),
  gitConnections: (provider: string) =>
    req<{ connections: GitConnection[] }>(`/api/v1/git/${provider}/connections`),
  gitRepos: (provider: string, connId: number) =>
    req<{ repositories: Repo[] }>(`/api/v1/git/${provider}/connections/${connId}/repos`),

  allowlist: () => req<{ rules: AllowlistRule[] }>("/api/v1/admin/allowlist"),
  addAllowlistRule: (type: string, value: string) =>
    req<AllowlistRule>("/api/v1/admin/allowlist", {
      method: "POST",
      body: JSON.stringify({ type, value }),
    }),
  deleteAllowlistRule: (id: number) =>
    req<void>(`/api/v1/admin/allowlist/${id}`, { method: "DELETE" }),

  nodes: () => req<{ nodes: RemoteNode[] }>("/api/v1/admin/nodes"),
  createNode: (body: { name: string; host: string; docker_url: string }) =>
    req<RemoteNode>("/api/v1/admin/nodes", { method: "POST", body: JSON.stringify(body) }),
  deleteNode: (id: string) => req<void>(`/api/v1/admin/nodes/${id}`, { method: "DELETE" }),
  checkNode: (id: string) =>
    req<RemoteNode>(`/api/v1/admin/nodes/${id}/health`, { method: "POST" }),

  teams: () => req<{ teams: Team[] }>("/api/v1/teams"),
  createTeam: (name: string) =>
    req<{ team: Team }>("/api/v1/teams", { method: "POST", body: JSON.stringify({ name }) }),
  team: (id: string) => req<{ team: TeamDetail }>(`/api/v1/teams/${id}`),
  renameTeam: (id: string, name: string) =>
    req<{ ok: boolean }>(`/api/v1/teams/${id}`, {
      method: "PATCH",
      body: JSON.stringify({ name }),
    }),
  deleteTeam: (id: string) => req<void>(`/api/v1/teams/${id}`, { method: "DELETE" }),
  updateMember: (teamId: string, userId: number, role: string) =>
    req<{ ok: boolean }>(`/api/v1/teams/${teamId}/members/${userId}`, {
      method: "PATCH",
      body: JSON.stringify({ role }),
    }),
  removeMember: (teamId: string, userId: number) =>
    req<void>(`/api/v1/teams/${teamId}/members/${userId}`, { method: "DELETE" }),
  invites: (teamId: string) =>
    req<{ invites: TeamInvite[] }>(`/api/v1/teams/${teamId}/invites`),
  createInvite: (teamId: string, email: string, role = "member") =>
    req<{ invite: TeamInvite }>(`/api/v1/teams/${teamId}/invites`, {
      method: "POST",
      body: JSON.stringify({ email, role }),
    }),
  revokeInvite: (teamId: string, inviteId: string) =>
    req<{ ok: boolean }>(`/api/v1/teams/${teamId}/invites/${inviteId}`, { method: "DELETE" }),
  acceptInvite: (inviteId: string) =>
    req<{ ok: boolean; team_id: string }>(`/api/v1/invites/${inviteId}/accept`, {
      method: "POST",
    }),
  teamAudit: (teamId: string) =>
    req<{ entries: AuditEntry[] }>(`/api/v1/teams/${teamId}/audit`),
  teamWebhooks: (teamId: string) =>
    req<{ webhooks: TeamWebhook[] }>(`/api/v1/teams/${teamId}/webhooks`),
  createTeamWebhook: (
    teamId: string,
    body: { name: string; url: string; secret?: string; events?: string[] },
  ) =>
    req<TeamWebhook>(`/api/v1/teams/${teamId}/webhooks`, {
      method: "POST",
      body: JSON.stringify(body),
    }),
  deleteTeamWebhook: (teamId: string, webhookId: string) =>
    req<void>(`/api/v1/teams/${teamId}/webhooks/${webhookId}`, { method: "DELETE" }),

  storage: (teamId: string) =>
    req<{ storage: Storage[] }>(`/api/v1/teams/${teamId}/storage`),
  createStorage: (teamId: string, body: { name: string; type: string; engine?: string }) =>
    req<void>(`/api/v1/teams/${teamId}/storage`, { method: "POST", body: JSON.stringify(body) }),
  deleteStorage: (teamId: string, storageId: string) =>
    req<void>(`/api/v1/teams/${teamId}/storage/${storageId}`, { method: "DELETE" }),
  resetStorage: (teamId: string, storageId: string) =>
    req<{ ok: boolean }>(`/api/v1/teams/${teamId}/storage/${storageId}/reset`, {
      method: "POST",
    }),
  linkStorage: (teamId: string, storageId: string, projectId: string) =>
    req<{ ok: boolean }>(`/api/v1/teams/${teamId}/storage/${storageId}/link`, {
      method: "POST",
      body: JSON.stringify({ project_id: projectId }),
    }),
  unlinkStorage: (teamId: string, storageId: string, projectId: string) =>
    req<void>(`/api/v1/teams/${teamId}/storage/${storageId}/link/${projectId}`, {
      method: "DELETE",
    }),

  notifications: () =>
    req<{ notifications: AppNotification[]; unread: number }>("/api/v1/notifications"),
  markAllRead: () =>
    req<{ ok: boolean }>("/api/v1/notifications/mark-read", { method: "POST" }),
};

export function deploymentLogsStream(id: string): EventSource {
  return new EventSource(`/api/v1/deployments/${id}/logs/stream`);
}

export function projectEventsStream(projectId: string): EventSource {
  return new EventSource(`/api/v1/projects/${projectId}/events`);
}
