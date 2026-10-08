import { api } from "@/lib/api";
import { Button, Input } from "@/components/ui";
import { Eye, EyeOff, GitBranch, KeyRound, Rocket, Server } from "lucide-react";
import { useEffect, useRef, useState } from "react";

const POINTS = [
  { icon: Rocket, text: "Push to deploy — builds, previews, rollbacks" },
  { icon: Server, text: "Your hardware, your rules — no per-seat pricing" },
  { icon: GitBranch, text: "GitHub, GitLab, Gitea, Bitbucket, and uploads" },
];

export default function Login() {
  const [mode, setMode] = useState<"login" | "register">("login");
  const [email, setEmail] = useState("");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [showPw, setShowPw] = useState(false);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [oidc, setOidc] = useState<{ enabled: boolean; display_name: string | null } | null>(null);
  const emailRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    api.oidcInfo().then(setOidc).catch(() => {});
    emailRef.current?.focus();
  }, []);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    setBusy(true);
    try {
      await (mode === "login"
        ? api.login(email, password)
        : api.register(email, password, username || undefined));
      window.location.href = "/projects";
    } catch (err) {
      setError(err instanceof Error ? err.message : "failed");
      setBusy(false);
    }
  }

  const label = "mb-1.5 block text-xs font-medium text-muted-foreground";

  return (
    <div className="grid min-h-screen lg:grid-cols-[1.1fr_1fr]">
      {/* Brand panel */}
      <div className="relative hidden flex-col justify-between overflow-hidden border-r border-border bg-gradient-to-b from-zinc-900 to-zinc-950 p-10 lg:flex">
        <div
          className="pointer-events-none absolute inset-0 opacity-[0.05]"
          style={{
            backgroundImage:
              "repeating-linear-gradient(0deg, transparent, transparent 47px, #fff 48px), repeating-linear-gradient(90deg, transparent, transparent 47px, #fff 48px)",
          }}
        />
        <div className="pointer-events-none absolute -top-1/4 right-0 h-[150%] w-72 rotate-[24deg] bg-gradient-to-b from-transparent via-white/[0.05] to-transparent" />
        <div className="relative flex items-center gap-3">
          <img src="/runway-mark-white.svg" alt="" className="h-9 w-9" />
          <span className="text-lg font-semibold tracking-tight">Runway</span>
        </div>
        <div className="relative">
          <img src="/runway-mark-white.svg" alt="" className="mb-8 h-40 w-40" />
          <h2 className="mb-3 text-3xl font-semibold tracking-tight">
            Ship it. From your own runway.
          </h2>
          <p className="mb-8 max-w-md text-sm leading-relaxed text-muted-foreground">
            A self-hosted deployment platform — push code, get a URL. Previews,
            rollbacks, domains, and HTTPS on hardware you control.
          </p>
          <ul className="space-y-3">
            {POINTS.map(({ icon: Icon, text }) => (
              <li key={text} className="flex items-center gap-3 text-sm text-muted-foreground">
                <Icon className="h-4 w-4 shrink-0 text-zinc-400" />
                {text}
              </li>
            ))}
          </ul>
        </div>
        <p className="relative text-xs text-zinc-600">Self-hosted · Single binary · MIT</p>
      </div>

      {/* Form panel */}
      <div className="flex items-center justify-center p-6">
        <div className="w-full max-w-sm">
          <div className="mb-8 flex items-center gap-3 lg:hidden">
            <img src="/runway-mark-white.svg" alt="" className="h-9 w-9" />
            <span className="text-lg font-semibold tracking-tight">Runway</span>
          </div>

          <h1 className="mb-1 text-2xl font-semibold tracking-tight">
            {mode === "login" ? "Welcome back" : "Create your account"}
          </h1>
          <p className="mb-6 text-sm text-muted-foreground">
            {mode === "login"
              ? "Sign in to your instance"
              : "The first account becomes the instance admin"}
          </p>

          <div className="mb-6 grid grid-cols-2 gap-1 rounded-lg border border-border p-1">
            {(["login", "register"] as const).map((m) => (
              <button
                key={m}
                type="button"
                onClick={() => {
                  setMode(m);
                  setError("");
                }}
                className={`rounded-md px-3 py-1.5 text-sm transition-colors ${
                  mode === m
                    ? "bg-zinc-800 font-medium text-foreground"
                    : "text-muted-foreground hover:text-foreground"
                }`}
              >
                {m === "login" ? "Sign in" : "Register"}
              </button>
            ))}
          </div>

          <form onSubmit={submit} className="space-y-4">
            <div>
              <label htmlFor="email" className={label}>
                Email
              </label>
              <Input
                id="email"
                ref={emailRef}
                type="email"
                required
                autoComplete="email"
                placeholder="you@example.com"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
              />
            </div>
            {mode === "register" && (
              <div>
                <label htmlFor="username" className={label}>
                  Username <span className="text-zinc-600">(optional)</span>
                </label>
                <Input
                  id="username"
                  autoComplete="username"
                  placeholder="your name"
                  value={username}
                  onChange={(e) => setUsername(e.target.value)}
                />
              </div>
            )}
            <div>
              <label htmlFor="password" className={label}>
                Password
              </label>
              <div className="relative">
                <Input
                  id="password"
                  type={showPw ? "text" : "password"}
                  required
                  minLength={8}
                  autoComplete={mode === "login" ? "current-password" : "new-password"}
                  placeholder={mode === "register" ? "8+ characters" : "password"}
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  className="pr-10"
                />
                <button
                  type="button"
                  aria-label={showPw ? "Hide password" : "Show password"}
                  onClick={() => setShowPw(!showPw)}
                  className="absolute right-3 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                >
                  {showPw ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
                </button>
              </div>
            </div>
            {error && <p className="text-xs text-destructive">{error}</p>}
            <Button type="submit" className="w-full" disabled={busy}>
              {busy ? "…" : mode === "login" ? "Sign in" : "Create account"}
            </Button>
          </form>

          {oidc?.enabled && (
            <>
              <div className="my-6 flex items-center gap-3 text-xs text-muted-foreground">
                <div className="h-px flex-1 bg-border" /> or <div className="h-px flex-1 bg-border" />
              </div>
              <a href="/api/auth/oidc" className="block">
                <Button variant="outline" className="w-full">
                  <KeyRound className="h-4 w-4" /> Continue with {oidc.display_name || "SSO"}
                </Button>
              </a>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
