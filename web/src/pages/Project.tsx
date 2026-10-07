import { api, projectEventsStream, type Deployment } from "@/lib/api";
import { Badge, Card, statusVariant } from "@/components/ui";
import { useEffect, useState } from "react";
import { Link, useParams } from "react-router-dom";

export default function ProjectPage() {
  const { id = "" } = useParams();
  const [deployments, setDeployments] = useState<Deployment[]>([]);
  const [error, setError] = useState("");

  useEffect(() => {
    const load = () =>
      api.deployments(id).then((r) => setDeployments(r.deployments)).catch((e) => setError(e.message));
    load();
    // Live updates: refresh on any deployment status event.
    const es = projectEventsStream(id);
    es.onmessage = () => load();
    return () => es.close();
  }, [id]);

  return (
    <div className="mx-auto max-w-5xl p-8">
      <h1 className="mb-6 text-xl font-semibold">Deployments</h1>
      {error && <p className="text-sm text-destructive">{error}</p>}
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
    </div>
  );
}
