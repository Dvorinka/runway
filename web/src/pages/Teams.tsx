import { api, type Team } from "@/lib/api";
import { Badge, Button, Card, Input } from "@/components/ui";
import { useEffect, useState } from "react";
import { Link } from "react-router-dom";

export default function TeamsPage() {
  const [teams, setTeams] = useState<Team[]>([]);
  const [name, setName] = useState("");
  const [error, setError] = useState("");

  const load = () => api.teams().then((r) => setTeams(r.teams)).catch((e) => setError(e.message));
  useEffect(() => {
    load();
  }, []);

  async function create(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    try {
      await api.createTeam(name);
      setName("");
      load();
    } catch (err) {
      setError(err instanceof Error ? err.message : "failed");
    }
  }

  return (
    <div className="page-enter mx-auto max-w-5xl p-4 sm:p-8">
      <div className="mb-6 flex items-center justify-between">
        <h1 className="text-xl font-semibold">Teams</h1>
        <form onSubmit={create} className="flex gap-2">
          <Input
            placeholder="new team name"
            value={name}
            onChange={(e) => setName(e.target.value)}
            required
          />
          <Button type="submit" size="sm">
            Create team
          </Button>
        </form>
      </div>
      {error && <p className="mb-4 text-xs text-destructive">{error}</p>}
      <div className="stagger grid gap-2">
        {teams.map((t) => (
          <Link key={t.id} to={`/teams/${t.id}`}>
            <Card className="flex items-center justify-between p-4 transition-colors hover:border-muted-foreground/25 hover:bg-accent/50">
              <div>
                <div className="text-sm font-medium">{t.name}</div>
                <div className="text-xs text-muted-foreground">{t.slug}</div>
              </div>
              <Badge variant={t.role === "owner" ? "default" : "secondary"}>{t.role}</Badge>
            </Card>
          </Link>
        ))}
        {teams.length === 0 && (
          <p className="text-sm text-muted-foreground">No teams yet — create one above.</p>
        )}
      </div>
    </div>
  );
}
