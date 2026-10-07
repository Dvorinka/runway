import { api, type Me } from "@/lib/api";
import Layout from "@/components/Layout";
import Login from "@/pages/Login";
import Projects from "@/pages/Projects";
import ProjectPage from "@/pages/Project";
import DeploymentPage from "@/pages/Deployment";
import SettingsPage from "@/pages/Settings";
import { useEffect, useState } from "react";
import { BrowserRouter, Navigate, Route, Routes } from "react-router-dom";

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
        <Route path="/" element={<Navigate to="/projects" replace />} />
        <Route path="/projects" element={page(<Projects />)} />
        <Route path="/projects/:id" element={page(<ProjectPage />)} />
        <Route path="/deployments/:id" element={page(<DeploymentPage />)} />
        <Route path="/settings" element={page(<SettingsPage me={me} />)} />
        <Route path="*" element={<Navigate to="/projects" replace />} />
      </Routes>
    </BrowserRouter>
  );
}
