import { api } from "@/lib/api";
import { Button, Input } from "@/components/ui";
import { Eye, EyeOff, GitBranch, KeyRound, Mail, Rocket, Server } from "lucide-react";
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
  const [magic, setMagic] = useState(false);
  const [oauth, setOauth] = useState({ github: false, google: false });
  const [sent, setSent] = useState(false);
  const [pending, setPending] = useState("");
  const [code, setCode] = useState("");
  const emailRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    api.oidcInfo().then(setOidc).catch(() => {});
    api.providers()
      .then((p) => {
        setMagic(p.magic_link);
        setOauth({ github: p.github, google: p.google });
      })
      .catch(() => {});
    if (new URLSearchParams(location.search).get("error") === "invalid_link") {
      setError("That sign-in link is invalid or has expired.");
    }
    emailRef.current?.focus();
  }, []);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setError("");
    setBusy(true);
    try {
      if (pending) {
        await api.totpChallenge(pending, code);
      } else if (mode === "register") {
        await api.register(email, password, username || undefined);
      } else {
        const res = await api.login(email, password);
        if (res.two_factor && res.pending) {
          setPending(res.pending);
          setBusy(false);
          return;
        }
      }
      window.location.href = "/";
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
          <div className="pointer-events-none absolute -left-24 top-1/3 h-96 w-96 rounded-full bg-brand/15 blur-[120px]" />
          <img src="/runway-mark-white.svg" alt="" className="relative mb-8 h-40 w-40" />
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

          {pending && (
            <p className="mb-6 rounded-md border border-border bg-muted/30 px-3 py-2 text-xs text-muted-foreground">
              Two-factor is enabled — enter the 6-digit code from your authenticator,
              or a recovery code.
            </p>
          )}
          {!pending && (
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
                    ? "border border-brand/40 bg-brand/15 font-medium text-brand"
                    : "border border-transparent text-muted-foreground hover:text-foreground"
                }`}
              >
                {m === "login" ? "Sign in" : "Register"}
              </button>
            ))}
          </div>
          )}

          <form onSubmit={submit} className="space-y-4">
            {pending ? (
              <div>
                <label htmlFor="totp" className={label}>
                  Two-factor code
                </label>
                <Input
                  id="totp"
                  required
                  autoFocus
                  autoComplete="one-time-code"
                  placeholder="000000 or recovery code"
                  value={code}
                  onChange={(e) => setCode(e.target.value)}
                  className="font-mono"
                />
              </div>
            ) : (
            <>
            <div>
              <label htmlFor="email" className={label}>
                Email
              </label>
              <Input
                id="email"
                name="email"
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
                  name="username"
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
                {/* key remounts on tab switch: password managers classify the
                    field at insert time and miss React-only autocomplete
                    flips (login fills, register generation never appears). */}
                <Input
                  id="password"
                  key={mode}
                  name="password"
                  type={showPw ? "text" : "password"}
                  required
                  autoComplete={mode === "login" ? "current-password" : "new-password"}
                  placeholder={mode === "register" ? "Choose a password" : "password"}
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
            </>
            )}
            {error && <p className="text-xs text-destructive">{error}</p>}
            {sent && (
              <p className="rounded-md border border-border bg-muted/30 px-3 py-2 text-xs text-muted-foreground">
                If that address has an account, a sign-in link is on its way.
              </p>
            )}
            <Button type="submit" className="w-full bg-brand text-white hover:bg-brand/90" disabled={busy}>
              {busy ? "…" : pending ? "Verify" : mode === "login" ? "Sign in" : "Create account"}
            </Button>
          </form>

          {(oidc?.enabled || oauth.github || oauth.google || (magic && mode === "login")) && (
            <div className="my-6 flex items-center gap-3 text-xs text-muted-foreground">
              <div className="h-px flex-1 bg-border" /> or <div className="h-px flex-1 bg-border" />
            </div>
          )}
          <div className="space-y-3">
            {oidc?.enabled && (
              <a href="/api/auth/oidc" className="block">
                <Button variant="outline" className="w-full">
                  <KeyRound className="h-4 w-4" /> Continue with {oidc.display_name || "SSO"}
                </Button>
              </a>
            )}
            {oauth.github && (
              <a href="/api/auth/oauth/github" className="block">
                <Button variant="outline" className="w-full">
                  <GitBranch className="h-4 w-4" /> Continue with GitHub
                </Button>
              </a>
            )}
            {oauth.google && (
              <a href="/api/auth/oauth/google" className="block">
                <Button variant="outline" className="w-full">
                  <KeyRound className="h-4 w-4" /> Continue with Google
                </Button>
              </a>
            )}
          </div>
          {magic && mode === "login" && !pending && (
            <Button
              variant="outline"
              className={`w-full ${oidc?.enabled || oauth.github || oauth.google ? "mt-3" : ""}`}
              disabled={busy || !email.includes("@")}
              onClick={async () => {
                setError("");
                setBusy(true);
                try {
                  await api.magicLink(email);
                  setSent(true);
                } catch (err) {
                  setError(err instanceof Error ? err.message : "failed");
                } finally {
                  setBusy(false);
                }
              }}
            >
              <Mail className="h-4 w-4" /> Email me a sign-in link
            </Button>
          )}
        </div>
      </div>
    </div>
  );
}
