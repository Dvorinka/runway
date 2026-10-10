import { api, type Me } from "@/lib/api";
import { Avatar, Button } from "@/components/ui";
import { CommandPalette } from "@/components/CommandPalette";
import { cn } from "@/lib/utils";
import { useEffect, useState } from "react";
import { Link, NavLink, useNavigate } from "react-router-dom";

function NavItem({ to, children }: { to: string; children: React.ReactNode }) {
  return (
    <NavLink to={to} className="relative flex items-center px-1 py-1 text-sm">
      {({ isActive }) => (
        <>
          <span
            className={cn(
              "transition-colors",
              isActive ? "text-foreground" : "text-muted-foreground hover:text-foreground",
            )}
          >
            {children}
          </span>
          <span
            className={cn(
              "absolute inset-x-0 -bottom-[13px] h-0.5 rounded-full bg-brand transition-opacity",
              isActive ? "opacity-100" : "opacity-0",
            )}
          />
        </>
      )}
    </NavLink>
  );
}

export default function Layout({ me, children }: { me: Me; children: React.ReactNode }) {
  const nav = useNavigate();
  const [unread, setUnread] = useState(0);
  useEffect(() => {
    const poll = () =>
      api
        .notifications()
        .then((r) => setUnread(r.unread))
        .catch(() => {});
    poll();
    const t = setInterval(poll, 30_000);
    return () => clearInterval(t);
  }, []);

  return (
    <div className="min-h-screen">
      <header className="sticky top-0 z-40 border-b border-border bg-background/80 backdrop-blur-md">
        <div className="mx-auto flex h-14 max-w-5xl items-center justify-between px-8">
          <div className="flex items-center gap-6">
            <Link to="/" className="flex items-center gap-2.5 font-semibold tracking-tight">
              <img src="/runway-mark-white.svg" alt="" className="h-5 w-5" />
              Runway
            </Link>
            <nav className="flex items-center gap-5">
              <NavItem to="/">Overview</NavItem>
              <NavItem to="/projects">Projects</NavItem>
              <NavItem to="/deployments">Deployments</NavItem>
              <NavItem to="/teams">Teams</NavItem>
              <NavItem to="/notifications">
                <span className="flex items-center gap-1.5">
                  Notifications
                  {unread > 0 && (
                    <span className="rounded-full bg-brand px-1.5 text-[11px] leading-4 font-medium text-white">
                      {unread}
                    </span>
                  )}
                </span>
              </NavItem>
              <NavItem to="/settings">Settings</NavItem>
            </nav>
          </div>
          <div className="flex items-center gap-3">
            <Avatar
              kind="user"
              id={me.id}
              name={me.name ?? me.username ?? me.email}
              hasAvatar={me.has_avatar}
            />
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
      <CommandPalette />
    </div>
  );
}
