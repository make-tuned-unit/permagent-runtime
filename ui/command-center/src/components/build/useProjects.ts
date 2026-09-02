import { useState, useEffect, useCallback } from 'react';
import { apiFetch } from '../../lib/api';

export interface Project {
  id: string;
  slug: string;
  name: string;
  description: string;
  status: string;
  rootPath: string | null;
  siteUrl: string | null;
  repoUrl: string | null;
  notes: string;
  tags: string[];
  createdAt: string;
  updatedAt: string;
  lastOpenedAt: string;
}

export function useProjects() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [loading, setLoading] = useState(true);
  // A failed fetch is not an empty project list. Swallowing it here is what
  // let the Build header's only launch affordance disappear on a daemon
  // outage with nothing on screen saying so.
  const [error, setError] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const data = await apiFetch<Project[]>('/api/projects?status=active');
      setProjects(data);
      setError(false);
      return true;
    } catch {
      setError(true);
      return false;
    } finally {
      setLoading(false);
    }
  }, []);

  const retry = useCallback(() => {
    setLoading(true);
    return refresh();
  }, [refresh]);

  useEffect(() => { refresh(); }, [refresh]);

  const touch = useCallback(async (id: string) => {
    try {
      await apiFetch(`/api/projects/${id}/touch`, { method: 'POST' });
    } catch {
      // best effort
    }
  }, []);

  return { projects, loading, error, refresh, retry, touch };
}
