import { api, type Team } from "@/lib/api";
import { Avatar, Badge, Button, Card, Input } from "@/components/ui";
import { Users } from "lucide-react";
import { useEffect, useState } from "react";
import { Link } from "react-router-dom";

export default function TeamsPage() {
  const [teams, setTeams] = useState<Team[]>([]);
  const [name, setName] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  const load = () => api.teams().then((r) => setTeams(r.teams)).catch((e) => setError(e.message));
  useEffect(() => {
    load();
  }, []);

  async function create(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    setBusy(true);
    try {
      await api.createTeam(name);
      setName("");
      await load();
    } catch (err) {
      setError(err instanceof Error ? err.message : "failed");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="page-enter mx-auto max-w-5xl p-4 sm:p-8">
      <div className="mb-6 flex flex-wrap items-end justify-between gap-3">
        <div>
          <h1 className="text-xl font-semibold">Teams</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            People, roles, shared storage, and webhooks per team.
          </p>
        </div>
        <form onSubmit={create} className="flex gap-2">
          <Input
            placeholder="new team name"
            value={name}
            onChange={(e) => setName(e.target.value)}
            required
            disabled={busy}
          />
          <Button type="submit" size="sm" disabled={busy}>
            {busy ? "Creating…" : "Create team"}
          </Button>
        </form>
      </div>
      {error && <p className="mb-4 text-xs text-destructive">{error}</p>}
      <div className="stagger grid gap-2">
        {teams.map((t) => (
          <Link key={t.id} to={`/teams/${t.id}`}>
            <Card className="flex items-center justify-between p-4 transition-colors hover:border-brand/25 hover:bg-accent/50">
              <div className="flex items-center gap-3">
                <Avatar kind="team" id={t.id} name={t.name} hasAvatar={t.has_avatar} />
                <div>
                  <div className="text-sm font-medium">{t.name}</div>
                  <div className="text-xs text-muted-foreground">{t.slug}</div>
                </div>
              </div>
              <Badge variant={t.role === "owner" ? "brand" : t.role === "admin" ? "default" : "secondary"}>{t.role}</Badge>
            </Card>
          </Link>
        ))}
        {teams.length === 0 && (
          <div className="rounded-lg border border-dashed border-border p-12 text-center">
            <div className="mx-auto mb-4 flex h-11 w-11 items-center justify-center rounded-full border border-border bg-accent/50">
              <Users className="h-5 w-5 text-muted-foreground" />
            </div>
            <p className="text-sm text-muted-foreground">No teams yet — create one above.</p>
          </div>
        )}
      </div>
    </div>
  );
}
