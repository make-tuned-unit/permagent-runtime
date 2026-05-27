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

  const refresh = useCallback(async () => {
    try {
      const data = await apiFetch<Project[]>('/api/projects?status=active');
      setProjects(data);
    } catch {
      // silently fail — projects feature may not be available
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { refresh(); }, [refresh]);

  const touch = useCallback(async (id: string) => {
    try {
      await apiFetch(`/api/projects/${id}/touch`, { method: 'POST' });
    } catch {
      // best effort
    }
  }, []);

  return { projects, loading, refresh, touch };
}
