import { api, type Me } from "@/lib/api";
import Layout from "@/components/Layout";
import Login from "@/pages/Login";
import Overview from "@/pages/Overview";
import Projects from "@/pages/Projects";
import DeploymentsPage from "@/pages/Deployments";
import ProjectPage from "@/pages/Project";
import DeploymentPage from "@/pages/Deployment";
import SettingsPage from "@/pages/Settings";
import TeamsPage from "@/pages/Teams";
import TeamPage from "@/pages/Team";
import NotificationsPage from "@/pages/Notifications";
import InvitePage from "@/pages/Invite";
import NotFound from "@/pages/NotFound";
import { useEffect, useState } from "react";
import { BrowserRouter, Route, Routes } from "react-router-dom";

export default function App() {
  const [me, setMe] = useState<Me | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    api
      .me()
      .then(setMe)
      .catch(() => setMe(null))
      .finally(() => setLoading(false));
  }, []);

  if (loading) return null;
  if (!me) return <Login />;

  const page = (el: React.ReactNode) => <Layout me={me}>{el}</Layout>;

  return (
    <BrowserRouter>
      <Routes>
        <Route path="/" element={page(<Overview />)} />
        <Route path="/projects" element={page(<Projects />)} />
        <Route path="/projects/:id" element={page(<ProjectPage />)} />
        <Route path="/deployments" element={page(<DeploymentsPage />)} />
        <Route path="/deployments/:id" element={page(<DeploymentPage />)} />
        <Route path="/teams" element={page(<TeamsPage />)} />
        <Route path="/teams/:id" element={page(<TeamPage me={me} />)} />
        <Route path="/invites/:id" element={page(<InvitePage />)} />
        <Route path="/notifications" element={page(<NotificationsPage />)} />
        <Route path="/settings" element={page(<SettingsPage me={me} />)} />
        <Route path="*" element={page(<NotFound />)} />
      </Routes>
    </BrowserRouter>
  );
}
