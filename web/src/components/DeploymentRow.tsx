import type { Deployment } from "@/lib/api";
import { Badge, Card, StatusDot, TimeAgo, displayStatus } from "@/components/ui";
import { duration, firstLine } from "@/lib/utils";
import { Link } from "react-router-dom";

export function DeploymentRow({ d, showProject }: { d: Deployment; showProject?: boolean }) {
  return (
    <Link to={`/deployments/${d.id}`}>
      <Card className="flex items-center justify-between gap-4 p-4 transition-colors hover:border-brand/25 hover:bg-accent/50">
        <div className="flex min-w-0 items-center gap-4">
          <StatusDot status={d.status} conclusion={d.conclusion} computed={d.computed_status} />
          <div className="min-w-0">
            <div className="truncate text-sm">
              {showProject && d.project_name && (
                <>
                  <span className="font-medium">{d.project_name}</span>
                  <span className="mx-2 text-muted-foreground/40">·</span>
                </>
              )}
              <span className="font-mono">{d.commit_sha.slice(0, 7)}</span>
              {firstLine(d.commit_meta?.message) && (
                <span className="ml-2 text-muted-foreground">
                  {firstLine(d.commit_meta?.message)}
                </span>
              )}
            </div>
            <div className="text-xs text-muted-foreground">
              {d.branch} · {d.environment_id}
              {d.commit_meta?.author && ` · ${d.commit_meta.author}`}
            </div>
          </div>
          <Badge variant={displayStatus(d).variant} className="shrink-0">
            {displayStatus(d).label}
          </Badge>
        </div>
        <div className="shrink-0 text-right text-xs text-muted-foreground">
          <TimeAgo at={d.created_at} />
          {duration(d.created_at, d.concluded_at) && (
            <div className="font-mono">{duration(d.created_at, d.concluded_at)}</div>
          )}
        </div>
      </Card>
    </Link>
  );
}
