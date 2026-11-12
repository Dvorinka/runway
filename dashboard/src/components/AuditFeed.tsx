import type { AuditEntry } from "@/lib/api";
import { timeAgo } from "@/lib/utils";
import { Link } from "react-router-dom";

const ACTION_LABEL: Record<string, string> = {
  "deployment.create": "Started a deployment",
  "project.delete": "Deleted a project",
  "team.create": "Created a team",
  "team.delete": "Deleted a team",
  "member.role": "Changed a member role",
  "member.remove": "Removed a member",
  "invite.create": "Sent an invite",
  "invite.accept": "Accepted an invite",
  "storage.create": "Created storage",
  "storage.delete": "Deleted storage",
};

export function actionLabel(action: string): string {
  return ACTION_LABEL[action] ?? action;
}

export const AUDIT_ACTIONS = Object.keys(ACTION_LABEL);

export function AuditFeed({ entries }: { entries: AuditEntry[] }) {
  return (
    <ul className="divide-y divide-border">
      {entries.map((e) => (
        <li key={e.id} data-testid="audit-row" className="flex gap-3 py-3">
          <time
            dateTime={e.created_at}
            className="w-16 shrink-0 pt-0.5 text-xs text-muted-foreground"
            title={new Date(e.created_at).toLocaleString()}
          >
            {timeAgo(e.created_at)}
          </time>
          <div className="min-w-0 flex-1">
            <div className="text-sm">{actionLabel(e.action)}</div>
            <div className="mt-0.5 flex min-w-0 flex-wrap items-center gap-x-2 text-xs text-muted-foreground">
              <span>{e.username ?? "system"}</span>
              {e.team_id && e.team_name && (
                <Link to={`/teams/${e.team_id}`} className="hover:text-foreground">
                  {e.team_name}
                </Link>
              )}
              {e.project_id && (
                <Link to={`/projects/${e.project_id}`} className="hover:text-foreground">
                  {e.project_name || e.project_id.slice(0, 7)}
                </Link>
              )}
              {e.detail && <span className="min-w-0 truncate">{e.detail}</span>}
            </div>
          </div>
          <code className="hidden shrink-0 pt-0.5 font-mono text-[11px] text-muted-foreground sm:block">
            {e.action}
          </code>
        </li>
      ))}
    </ul>
  );
}
