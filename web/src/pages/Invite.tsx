import { api } from "@/lib/api";
import { Button, Card } from "@/components/ui";
import { useState } from "react";
import { useNavigate, useParams } from "react-router-dom";

export default function InvitePage() {
  const { id } = useParams<{ id: string }>();
  const nav = useNavigate();
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  async function accept() {
    if (!id) return;
    setBusy(true);
    setError("");
    try {
      const r = await api.acceptInvite(id);
      nav(`/teams/${r.team_id}`);
    } catch (e) {
      setError(e instanceof Error ? e.message : "failed");
      setBusy(false);
    }
  }

  return (
    <div className="page-enter mx-auto max-w-md pt-24">
      <Card className="p-6 text-center">
        <h1 className="mb-2 text-lg font-semibold">Team invite</h1>
        <p className="mb-6 text-sm text-muted-foreground">
          Accept to join the team. The invite is bound to your account email.
        </p>
        {error && <p className="mb-4 text-xs text-destructive">{error}</p>}
        <Button onClick={accept} disabled={busy}>
          {busy ? "Joining…" : "Accept invite"}
        </Button>
      </Card>
    </div>
  );
}
