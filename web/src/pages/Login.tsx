import { Button, Card, Input } from "@/components/ui";
import { Github } from "lucide-react";
import { useState } from "react";

export default function Login() {
  const [email, setEmail] = useState("");
  const [sent, setSent] = useState(false);
  const [error, setError] = useState("");

  async function magicLink(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    try {
      const res = await fetch("/api/auth/magic-link", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ email }),
      });
      if (!res.ok) throw new Error((await res.json()).error || "failed");
      setSent(true);
    } catch (err) {
      setError(err instanceof Error ? err.message : "failed to send link");
    }
  }

  return (
    <div className="flex min-h-screen items-center justify-center p-6">
      <Card className="w-full max-w-sm p-8">
        <h1 className="mb-1 text-2xl font-semibold tracking-tight">Runway</h1>
        <p className="mb-6 text-sm text-muted-foreground">Sign in to your instance</p>

        <a href="/api/auth/github" className="block">
          <Button variant="outline" className="w-full">
            <Github className="h-4 w-4" /> Continue with GitHub
          </Button>
        </a>

        <div className="my-5 flex items-center gap-3 text-xs text-muted-foreground">
          <div className="h-px flex-1 bg-border" /> or <div className="h-px flex-1 bg-border" />
        </div>

        {sent ? (
          <p className="text-sm text-muted-foreground">
            Check your inbox — the sign-in link was sent to <b>{email}</b>.
          </p>
        ) : (
          <form onSubmit={magicLink} className="space-y-3">
            <Input
              type="email"
              required
              placeholder="you@example.com"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
            />
            <Button type="submit" className="w-full">
              Email me a sign-in link
            </Button>
            {error && <p className="text-xs text-destructive">{error}</p>}
          </form>
        )}
      </Card>
    </div>
  );
}
