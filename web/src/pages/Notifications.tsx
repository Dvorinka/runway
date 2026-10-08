import { api, type AppNotification } from "@/lib/api";
import { Badge, Button, Card } from "@/components/ui";
import { timeAgo } from "@/lib/utils";
import { useEffect, useState } from "react";
import { Link } from "react-router-dom";

export default function NotificationsPage() {
  const [items, setItems] = useState<AppNotification[]>([]);
  const [unread, setUnread] = useState(0);
  const [error, setError] = useState("");

  const load = () =>
    api
      .notifications()
      .then((r) => {
        setItems(r.notifications);
        setUnread(r.unread);
      })
      .catch((e) => setError(e.message));

  useEffect(() => {
    load();
  }, []);

  return (
    <div className="page-enter mx-auto max-w-5xl p-8">
      <div className="mb-6 flex items-center justify-between">
        <h1 className="text-xl font-semibold">
          Notifications
          {unread > 0 && (
            <Badge variant="warning" className="ml-2">
              {unread} unread
            </Badge>
          )}
        </h1>
        {unread > 0 && (
          <Button size="sm" variant="outline" onClick={() => api.markAllRead().then(load)}>
            Mark all read
          </Button>
        )}
      </div>
      {error && <p className="mb-4 text-xs text-destructive">{error}</p>}
      <div className="stagger grid gap-2">
        {items.map((n) => {
          const inner = (
            <Card
              className={`flex items-center justify-between p-4 transition-colors hover:border-muted-foreground/25 hover:bg-accent/50 ${
                n.read ? "opacity-60" : ""
              }`}
            >
              <div className="flex items-start gap-3">
                {!n.read && <span className="mt-1.5 h-2 w-2 shrink-0 rounded-full bg-primary" />}
                <div>
                  <div className="text-sm">{n.title}</div>
                  {n.body && <div className="mt-1 text-xs text-muted-foreground">{n.body}</div>}
                </div>
              </div>
              <div className="shrink-0 text-xs text-muted-foreground">{timeAgo(n.created_at)}</div>
            </Card>
          );
          return n.link ? (
            <Link key={n.id} to={n.link}>
              {inner}
            </Link>
          ) : (
            <div key={n.id}>{inner}</div>
          );
        })}
        {items.length === 0 && (
          <p className="text-sm text-muted-foreground">Nothing yet — deploy events show up here.</p>
        )}
      </div>
    </div>
  );
}
