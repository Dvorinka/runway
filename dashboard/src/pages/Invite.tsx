import { api } from "@/lib/api";
import { Badge, Button, Card, Skeleton } from "@/components/ui";
import { useEffect, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";

type Preview = {
  team_name: string;
  email: string;
  role: string;
  inviter: string | null;
  expires_at: string;
  expired: boolean;
  yours: boolean;
};

export default function InvitePage() {
  const { id } = useParams<{ id: string }>();
  const nav = useNavigate();
  const [preview, setPreview] = useState<Preview | null | undefined>(undefined);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!id) return;
    api.invitePreview(id).then(setPreview).catch(() => setPreview(null));
  }, [id]);

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
        <img src="/runway-mark-white.svg" alt="" className="mx-auto mb-4 h-10 w-10" />
        {preview === undefined && <Skeleton className="mx-auto h-28 w-64" />}
        {preview === null && (
          <>
            <h1 className="mb-2 text-lg font-semibold">Invite unavailable</h1>
            <p className="mb-6 text-sm text-muted-foreground">
              This invite was revoked, already used, or never existed. Ask the team for a new link.
            </p>
          </>
        )}
        {preview && (
          <>
            <h1 className="mb-2 text-lg font-semibold">Join {preview.team_name}</h1>
            <p className="mb-4 text-sm text-muted-foreground">
              {preview.inviter ? `${preview.inviter} invited you` : "You've been invited"} to join as{" "}
              <Badge variant="outline" className="mx-1">{preview.role}</Badge>
              — the invite is bound to {preview.email}.
            </p>
            {preview.expired && (
              <p className="mb-4 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs text-destructive">
                This invite has expired. Ask the team for a new link.
              </p>
            )}
            {!preview.expired && !preview.yours && (
              <p className="mb-4 rounded-md border border-warning/40 bg-warning/10 px-3 py-2 text-xs text-warning">
                You're signed in with a different email — this invite won't accept for this account.
              </p>
            )}
            {error && <p className="mb-4 text-xs text-destructive">{error}</p>}
            <Button
              onClick={accept}
              disabled={busy || preview.expired || !preview.yours}
              className="bg-brand text-white hover:bg-brand/90"
            >
              {busy ? "Joining…" : `Accept invite`}
            </Button>
          </>
        )}
      </Card>
    </div>
  );
}
