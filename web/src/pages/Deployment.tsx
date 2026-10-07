import { api, deploymentLogsStream, type Deployment } from "@/lib/api";
import { Badge, Button, statusVariant } from "@/components/ui";
import { useEffect, useRef, useState } from "react";
import { useParams } from "react-router-dom";

export default function DeploymentPage() {
  const { id = "" } = useParams();
  const [dep, setDep] = useState<Deployment | null>(null);
  const [lines, setLines] = useState<string[]>([]);
  const logRef = useRef<HTMLPreElement>(null);

  useEffect(() => {
    api.deployment(id).then(setDep).catch(() => {});
    api.deploymentLogs(id).then((t) => setLines(t.split("\n").filter(Boolean)));

    const es = deploymentLogsStream(id);
    es.onmessage = (ev) =>
      setLines((prev) => [...prev.slice(-4000), ev.data as string]);
    return () => es.close();
  }, [id]);

  useEffect(() => {
    logRef.current?.scrollTo(0, logRef.current.scrollHeight);
  }, [lines]);

  const active = dep && !dep.conclusion;

  return (
    <div className="mx-auto flex h-screen max-w-5xl flex-col p-8">
      <div className="mb-4 flex items-center justify-between">
        <div className="flex items-center gap-3">
          <h1 className="font-mono text-lg">{dep?.commit_sha.slice(0, 7) ?? id.slice(0, 7)}</h1>
          {dep && (
            <Badge variant={statusVariant(dep.status, dep.conclusion)}>
              {dep.conclusion ?? dep.status}
            </Badge>
          )}
          {dep?.url && (
            <a href={dep.url} target="_blank" className="text-sm text-muted-foreground underline">
              {dep.url}
            </a>
          )}
        </div>
        <div className="flex gap-2">
          {active && (
            <Button variant="outline" size="sm" onClick={() => api.cancel(id).then(() => location.reload())}>
              Cancel
            </Button>
          )}
          <Button
            variant="outline"
            size="sm"
            onClick={() => api.redeploy(id).then((d) => (location.href = `/deployments/${d.id}`))}
          >
            Redeploy
          </Button>
        </div>
      </div>
      <pre
        ref={logRef}
        className="flex-1 overflow-auto rounded-lg border border-border bg-card p-4 font-mono text-xs leading-relaxed"
      >
        {lines.join("\n") || "waiting for logs…"}
      </pre>
    </div>
  );
}
