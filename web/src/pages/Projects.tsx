import { api, type Project } from "@/lib/api";
import { Badge, Card } from "@/components/ui";
import { useEffect, useState } from "react";
import { Link } from "react-router-dom";

export default function Projects() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [error, setError] = useState("");

  useEffect(() => {
    api.projects().then((r) => setProjects(r.projects)).catch((e) => setError(e.message));
  }, []);

  return (
    <div className="mx-auto max-w-5xl p-8">
      <h1 className="mb-6 text-xl font-semibold">Projects</h1>
      {error && <p className="text-sm text-destructive">{error}</p>}
      {projects.length === 0 && !error && (
        <p className="text-sm text-muted-foreground">
          No projects yet. Connect a GitHub repository to get started.
        </p>
      )}
      <div className="grid gap-3">
        {projects.map((p) => (
          <Link key={p.id} to={`/projects/${p.id}`}>
            <Card className="flex items-center justify-between p-4 transition-colors hover:bg-accent">
              <div>
                <div className="font-medium">{p.name}</div>
                <div className="text-xs text-muted-foreground">
                  {p.repo_full_name} · {p.repo_branch}
                </div>
              </div>
              <Badge variant={p.status === "active" ? "success" : "secondary"}>{p.status}</Badge>
            </Card>
          </Link>
        ))}
      </div>
    </div>
  );
}
