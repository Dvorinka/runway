export interface Project {
  id: string;
  team_id: string;
  name: string;
  slug: string | null;
  repo_full_name: string;
  repo_branch: string;
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
  return res.json() as Promise<T>;
}

export const api = {
  me: () => req<Me>("/api/auth/me"),
  logout: () => req<void>("/api/auth/logout", { method: "POST" }),
  projects: () => req<{ projects: Project[] }>("/api/v1/projects"),
  project: (id: string) => req<Project>(`/api/v1/projects/${id}`),
  deployments: (projectId: string) =>
    req<{ deployments: Deployment[] }>(`/api/v1/projects/${projectId}/deployments`),
  deployment: (id: string) => req<Deployment>(`/api/v1/deployments/${id}`),
  deploymentLogs: (id: string, tail = 500) =>
    fetch(`/api/v1/deployments/${id}/logs?tail=${tail}`, {
      credentials: "include",
    }).then((r) => r.text()),
  redeploy: (id: string) =>
    req<Deployment>(`/api/v1/deployments/${id}/redeploy`, { method: "POST" }),
  cancel: (id: string) =>
    req<Deployment>(`/api/v1/deployments/${id}/cancel`, { method: "POST" }),
};

export function deploymentLogsStream(id: string): EventSource {
  return new EventSource(`/api/v1/deployments/${id}/logs/stream`);
}

export function projectEventsStream(projectId: string): EventSource {
  return new EventSource(`/api/v1/projects/${projectId}/events`);
}
