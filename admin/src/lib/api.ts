// API client helper for server-side fetches

export async function fetchFromBackend(
  path: string,
  astroRequest: Request,
  options: RequestInit = {}
) {
  const backendUrl = import.meta.env.QUERIA_API_URL || process.env.QUERIA_API_URL || 'http://localhost:17671';

  // Forward cookie header from the client to the backend API for session validation
  const headers = new Headers(options.headers || {});
  const cookieHeader = astroRequest.headers.get('cookie');
  if (cookieHeader) {
    headers.set('cookie', cookieHeader);
  }

  // Ensure JSON requests set content-type
  if (options.body && !headers.has('content-type')) {
    headers.set('content-type', 'application/json');
  }

  try {
    const response = await fetch(`${backendUrl}${path}`, {
      ...options,
      headers,
    });
    return response;
  } catch (error) {
    console.error(`Backend fetch failed for path ${path}:`, error);
    throw error;
  }
}

export async function getDashboardSummary(astroRequest: Request) {
  const res = await fetchFromBackend('/api/v1/dashboard/summary', astroRequest);
  if (!res.ok) {
    if (res.status === 401) return null;
    throw new Error(`Failed to fetch dashboard summary: ${res.statusText}`);
  }
  return res.json();
}

export async function listProjects(astroRequest: Request) {
  const res = await fetchFromBackend('/api/v1/projects', astroRequest);
  if (!res.ok) {
    if (res.status === 401) return null;
    throw new Error(`Failed to fetch projects list: ${res.statusText}`);
  }
  return res.json();
}

export async function listSources(astroRequest: Request, projectSlug: string) {
  const res = await fetchFromBackend(`/api/v1/sources?project_slug=${projectSlug}`, astroRequest);
  if (!res.ok) {
    if (res.status === 401) return null;
    throw new Error(`Failed to fetch sources list: ${res.statusText}`);
  }
  return res.json();
}

export async function getSourceDetail(astroRequest: Request, sourceId: string) {
  const res = await fetchFromBackend(`/api/v1/sources/${sourceId}`, astroRequest);
  if (!res.ok) {
    if (res.status === 401) return null;
    throw new Error(`Failed to fetch source detail: ${res.statusText}`);
  }
  return res.json();
}

export async function listKnowledgeItems(
  astroRequest: Request,
  params: {
    scope?: string;
    projectSlug?: string;
    category?: string;
    status?: string;
    tag?: string;
    cursor?: string;
    limit?: number;
  } = {}
) {
  const searchParams = new URLSearchParams();
  if (params.scope) searchParams.set('scope', params.scope);
  if (params.projectSlug) searchParams.set('project_slug', params.projectSlug);
  if (params.category) searchParams.set('category', params.category);
  if (params.status) searchParams.set('status', params.status);
  if (params.tag) searchParams.set('tag', params.tag);
  if (params.cursor) searchParams.set('cursor', params.cursor);
  if (params.limit) searchParams.set('limit', params.limit.toString());

  const queryStr = searchParams.toString();
  const url = `/api/v1/knowledge-items${queryStr ? `?${queryStr}` : ''}`;

  const res = await fetchFromBackend(url, astroRequest);
  if (!res.ok) {
    if (res.status === 401) return null;
    throw new Error(`Failed to fetch knowledge items: ${res.statusText}`);
  }
  return res.json();
}

export async function getKnowledgeItemDetail(astroRequest: Request, itemId: string) {
  const res = await fetchFromBackend(`/api/v1/knowledge-items/${itemId}`, astroRequest);
  if (!res.ok) {
    if (res.status === 401) return null;
    throw new Error(`Failed to fetch knowledge item detail: ${res.statusText}`);
  }
  return res.json();
}

export async function listApprovals(astroRequest: Request, status?: string) {
  const url = `/api/v1/approvals${status ? `?status=${status}` : ''}`;
  const res = await fetchFromBackend(url, astroRequest);
  if (!res.ok) {
    if (res.status === 401) return null;
    throw new Error(`Failed to fetch approvals queue: ${res.statusText}`);
  }
  return res.json();
}

export async function listAgentTokens(astroRequest: Request) {
  const res = await fetchFromBackend('/api/v1/agent-tokens', astroRequest);
  if (!res.ok) {
    if (res.status === 401) return null;
    throw new Error(`Failed to fetch agent tokens: ${res.statusText}`);
  }
  return res.json();
}

export async function listAuditLogs(
  astroRequest: Request,
  params: {
    actor_id?: string;
    action?: string;
    resource_type?: string;
    resource_id?: string;
    cursor?: string;
    limit?: number;
  } = {}
) {
  const searchParams = new URLSearchParams();
  if (params.actor_id) searchParams.set('actor_id', params.actor_id);
  if (params.action) searchParams.set('action', params.action);
  if (params.resource_type) searchParams.set('resource_type', params.resource_type);
  if (params.resource_id) searchParams.set('resource_id', params.resource_id);
  if (params.cursor) searchParams.set('cursor', params.cursor);
  if (params.limit) searchParams.set('limit', params.limit.toString());

  const queryStr = searchParams.toString();
  const url = `/api/v1/audit-logs${queryStr ? `?${queryStr}` : ''}`;

  const res = await fetchFromBackend(url, astroRequest);
  if (!res.ok) {
    if (res.status === 401) return null;
    throw new Error(`Failed to fetch audit logs: ${res.statusText}`);
  }
  return res.json();
}

export async function listJobs(astroRequest: Request) {
  const res = await fetchFromBackend('/api/v1/ingestion-jobs', astroRequest);
  if (!res.ok) {
    if (res.status === 401) return null;
    throw new Error(`Failed to fetch ingestion jobs: ${res.statusText}`);
  }
  return res.json();
}

export async function createProject(
  astroRequest: Request,
  payload: {
    slug: string;
    name: string;
    description?: string;
    default_embedding_model?: string;
    include_global_default?: boolean;
  }
) {
  const res = await fetchFromBackend('/api/v1/projects', astroRequest, {
    method: 'POST',
    body: JSON.stringify(payload),
  });
  return res;
}

/** Operator retrieval probe: POST /api/v1/projects/{slug}/retrieval/probe */
export async function probeRetrieval(
  astroRequest: Request,
  projectSlug: string,
  payload: {
    query: string;
    include_global?: boolean;
    include_scratch?: boolean;
    limit?: number;
    rerank?: boolean;
    compress?: boolean;
  }
) {
  const res = await fetchFromBackend(
    `/api/v1/projects/${encodeURIComponent(projectSlug)}/retrieval/probe`,
    astroRequest,
    {
      method: 'POST',
      body: JSON.stringify(payload),
    }
  );
  return res;
}


