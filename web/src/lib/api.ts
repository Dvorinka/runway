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
  config: Record<string, unknown>;
  status: string;
  created_at: string;
}

export interface Deployment {
  id: string;
  project_id: string;
  environment_id: string;
  branch: string;
  commit_sha: string;
  status: string;
  conclusion: string | null;
  container_status: string | null;
  url?: string;
  created_at: string;
}

export interface Me {
  id: number;
  email: string;
  username: string;
  name: string | null;
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
  logout: () => req<void>("/api/auth/logout", { method: "POST" }),
  oidcInfo: () =>
    req<{ enabled: boolean; display_name: string | null }>("/api/auth/oidc/info"),

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

  getEnv: (id: string) => req<{ env: EnvVar[] }>(`/api/v1/projects/${id}/env`),
  patchEnv: (id: string, vars: { key: string; value?: string; delete?: boolean }[]) =>
    req<{ ok: boolean }>(`/api/v1/projects/${id}/env`, {
      method: "PATCH",
      body: JSON.stringify(vars),
    }),

  deployments: (projectId: string) =>
    req<{ deployments: Deployment[] }>(`/api/v1/projects/${projectId}/deployments`),
  deploy: (projectId: string, body: { branch?: string } = {}) =>
    req<Deployment>(`/api/v1/projects/${projectId}/deployments`, {
      method: "POST",
      body: JSON.stringify(body),
    }),
  deployment: (id: string) => req<Deployment>(`/api/v1/deployments/${id}`),
  deploymentLogs: (id: string, tail = 500) =>
    fetch(`/api/v1/deployments/${id}/logs?tail=${tail}`, {
      credentials: "include",
    }).then((r) => r.text()),
  redeploy: (id: string) =>
    req<Deployment>(`/api/v1/deployments/${id}/redeploy`, { method: "POST" }),
  cancel: (id: string) =>
    req<Deployment>(`/api/v1/deployments/${id}/cancel`, { method: "POST" }),

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
};

export function deploymentLogsStream(id: string): EventSource {
  return new EventSource(`/api/v1/deployments/${id}/logs/stream`);
}

export function projectEventsStream(projectId: string): EventSource {
  return new EventSource(`/api/v1/projects/${projectId}/events`);
}
