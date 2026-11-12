import { api, type AuditEntry, type Team } from "@/lib/api";
import { AuditFeed, AUDIT_ACTIONS, actionLabel } from "@/components/AuditFeed";
import { Card, ComboBox, EmptyState, Skeleton } from "@/components/ui";
import { ScrollText } from "lucide-react";
import { useEffect, useState } from "react";

export default function ActivityPage() {
  const [entries, setEntries] = useState<AuditEntry[] | null>(null);
  const [teams, setTeams] = useState<Team[]>([]);
  const [teamId, setTeamId] = useState("");
  const [action, setAction] = useState("");
  const [error, setError] = useState("");

  useEffect(() => {
    api.teams().then((r) => setTeams(r.teams)).catch(() => {});
  }, []);

  useEffect(() => {
    let live = true;
    setEntries(null);
    setError("");
    api
      .audit({
        team_id: teamId || undefined,
        action: action || undefined,
      })
      .then((r) => {
        if (live) setEntries(r.entries);
      })
      .catch((e) => {
        if (live) setError(e instanceof Error ? e.message : "failed");
      });
    return () => {
      live = false;
    };
  }, [teamId, action]);

  return (
    <div className="page-enter mx-auto max-w-5xl p-4 sm:p-8">
      <h1 className="text-xl font-semibold">Activity</h1>
      <p className="mb-6 mt-1 max-w-xl text-sm text-muted-foreground">
        Who changed what. Team admins see their teams. Members see their own
        actions. The instance admin sees every team.
      </p>
      <div className="mb-4 grid grid-cols-1 gap-2 sm:grid-cols-2">
        <ComboBox
          value={teamId}
          onChange={setTeamId}
          placeholder="All teams"
          searchPlaceholder="Filter teams…"
          options={[
            { value: "", label: "All teams" },
            ...teams.map((t) => ({ value: t.id, label: t.name, hint: t.role })),
          ]}
        />
        <ComboBox
          value={action}
          onChange={setAction}
          placeholder="All actions"
          searchPlaceholder="Filter actions…"
          options={[
            { value: "", label: "All actions" },
            ...AUDIT_ACTIONS.map((a) => ({ value: a, label: actionLabel(a), hint: a })),
          ]}
        />
      </div>
      {error && <p className="mb-4 text-sm text-destructive">{error}</p>}
      {entries === null && !error ? (
        <div className="grid grid-cols-1 gap-2">
          {[0, 1, 2, 3].map((i) => (
            <Skeleton key={i} className="h-14" />
          ))}
        </div>
      ) : entries && entries.length === 0 ? (
        <EmptyState
          icon={ScrollText}
          title="No activity yet"
          hint="Deploys, invites, and team changes you are allowed to see show up here."
        />
      ) : (
        entries && (
          <Card className="px-4">
            <AuditFeed entries={entries} />
          </Card>
        )
      )}
    </div>
  );
}
