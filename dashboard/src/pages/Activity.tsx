import { api, type AuditEntry, type Team } from "@/lib/api";
import { AuditFeed, AUDIT_ACTIONS, actionLabel } from "@/components/AuditFeed";
import { Card, ComboBox, EmptyState, Skeleton } from "@/components/ui";
import { ScrollText } from "lucide-react";
import { useEffect, useRef, useState } from "react";

const PAGE = 50;

function csvCell(value: string | null | undefined): string {
  const s = value ?? "";
  if (/[",\n]/.test(s)) return `"${s.replaceAll('"', '""')}"`;
  return s;
}

function downloadCsv(entries: AuditEntry[]) {
  const header = ["created_at", "action", "username", "team_name", "project_name", "detail"];
  const lines = [
    header.join(","),
    ...entries.map((e) =>
      [
        e.created_at,
        e.action,
        e.username,
        e.team_name,
        e.project_name,
        e.detail,
      ]
        .map(csvCell)
        .join(","),
    ),
  ];
  const blob = new Blob([lines.join("\n")], { type: "text/csv" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = "runway-activity.csv";
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

export default function ActivityPage() {
  const [entries, setEntries] = useState<AuditEntry[] | null>(null);
  const [hasMore, setHasMore] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [teams, setTeams] = useState<Team[]>([]);
  const [teamId, setTeamId] = useState("");
  const [action, setAction] = useState("");
  const [error, setError] = useState("");
  const filterKey = `${teamId}\0${action}`;
  const filterRef = useRef(filterKey);
  filterRef.current = filterKey;

  useEffect(() => {
    api.teams().then((r) => setTeams(r.teams)).catch(() => {});
  }, []);

  useEffect(() => {
    let live = true;
    setEntries(null);
    setHasMore(false);
    setError("");
    api
      .audit({
        team_id: teamId || undefined,
        action: action || undefined,
        limit: PAGE,
      })
      .then((r) => {
        if (!live) return;
        setEntries(r.entries);
        setHasMore(r.has_more);
      })
      .catch((e) => {
        if (live) setError(e instanceof Error ? e.message : "failed");
      });
    return () => {
      live = false;
    };
  }, [teamId, action]);

  function earlier() {
    const last = entries?.[entries.length - 1];
    if (!last || loadingMore) return;
    const key = filterKey;
    setLoadingMore(true);
    api
      .audit({
        team_id: teamId || undefined,
        action: action || undefined,
        limit: PAGE,
        before_id: last.id,
        before_at: last.created_at,
      })
      .then((r) => {
        if (filterRef.current !== key) return;
        setEntries((cur) => {
          const seen = new Set((cur ?? []).map((e) => e.id));
          return [...(cur ?? []), ...r.entries.filter((e) => !seen.has(e.id))];
        });
        setHasMore(r.has_more);
      })
      .catch((e) => {
        if (filterRef.current === key) setError(e instanceof Error ? e.message : "failed");
      })
      .finally(() => {
        if (filterRef.current === key) setLoadingMore(false);
      });
  }

  return (
    <div className="page-enter mx-auto max-w-5xl p-4 sm:p-8">
      <div className="mb-6 flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <h1 className="text-xl font-semibold">Activity</h1>
          <p className="mt-1 max-w-xl text-sm text-muted-foreground">
            Who changed what. Team admins see their teams. Members see their own
            actions. The instance admin sees every team.
          </p>
        </div>
        {entries && entries.length > 0 && (
          <button
            type="button"
            onClick={() => downloadCsv(entries)}
            className="self-start rounded-md border border-border px-2.5 py-1 text-xs text-muted-foreground transition-colors hover:text-foreground"
          >
            Download CSV
          </button>
        )}
      </div>
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
            {hasMore && (
              <div className="border-t border-border py-3">
                <button
                  type="button"
                  onClick={earlier}
                  disabled={loadingMore}
                  className="rounded-md border border-border px-2.5 py-1 text-xs text-muted-foreground transition-colors hover:text-foreground disabled:opacity-50"
                >
                  {loadingMore ? "Loading…" : "Earlier"}
                </button>
              </div>
            )}
          </Card>
        )
      )}
    </div>
  );
}
