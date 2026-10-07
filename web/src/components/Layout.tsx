import { api, type Me } from "@/lib/api";
import { Button } from "@/components/ui";
import { Link, NavLink, useNavigate } from "react-router-dom";

export default function Layout({ me, children }: { me: Me; children: React.ReactNode }) {
  const nav = useNavigate();
  const linkCls = ({ isActive }: { isActive: boolean }) =>
    `text-sm ${isActive ? "text-foreground" : "text-muted-foreground hover:text-foreground"}`;
  return (
    <div className="min-h-screen">
      <header className="border-b border-border">
        <div className="mx-auto flex max-w-5xl items-center justify-between px-8 py-3">
          <div className="flex items-center gap-6">
            <Link to="/projects" className="font-semibold tracking-tight">
              Runway
            </Link>
            <NavLink to="/projects" className={linkCls}>
              Projects
            </NavLink>
            <NavLink to="/settings" className={linkCls}>
              Settings
            </NavLink>
          </div>
          <div className="flex items-center gap-3">
            <span className="text-xs text-muted-foreground">{me.email}</span>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => api.logout().then(() => nav(0)).catch(() => nav(0))}
            >
              Sign out
            </Button>
          </div>
        </div>
      </header>
      {children}
    </div>
  );
}
