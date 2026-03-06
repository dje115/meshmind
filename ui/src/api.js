const DEFAULT_BASE = 'http://127.0.0.1:9900';
const API_PREFIX = '/v1';
// Use relative URL when: (a) Vite dev (proxy), or (b) page is already on API origin (e.g. backend-serving UI)
function resolveBaseUrl() {
  if (typeof import.meta !== 'undefined' && import.meta.env?.VITE_API_BASE_URL) {
    return import.meta.env.VITE_API_BASE_URL;
  }
  if (typeof import.meta !== 'undefined' && import.meta.env?.DEV) {
    return ''; // Vite dev - proxy forwards /v1
  }
  try {
    const host = typeof window !== 'undefined' ? window.location.host : '';
    if (host.includes('9900')) return ''; // Same-origin: backend serving UI on :9900
  } catch (_) {}
  return DEFAULT_BASE;
}
let baseUrl = resolveBaseUrl();
let adminToken = null;

export function getApiBase() { return baseUrl; }
export function setApiBase(url) {
  baseUrl = url || DEFAULT_BASE;
}

function getRequestUrl(path) {
  const base = baseUrl || '';
  return base ? `${base}${API_PREFIX}${path}` : `${API_PREFIX}${path}`;
}

async function request(method, path, body) {
  const opts = { method, headers: {} };
  if (adminToken && path.startsWith('/admin')) {
    opts.headers['Authorization'] = `Bearer ${adminToken}`;
  }
  if (body) {
    opts.headers['Content-Type'] = 'application/json';
    opts.body = JSON.stringify(body);
  }
  const url = getRequestUrl(path);
  const res = await fetch(url, opts);
  const contentType = res.headers.get('content-type') || '';
  const isJson = contentType.includes('application/json');

  if (!res.ok) {
    let message = `${res.status} ${res.statusText}`;
    if (isJson) {
      try {
        const err = await res.json();
        if (err && typeof err.error === 'string') message = err.error;
      } catch (_) {}
    } else if (contentType.includes('text/html')) {
      message = 'Server returned HTML instead of JSON. Is the MeshMind API running on ' + (baseUrl || 'the configured host') + '?';
    }
    throw new Error(message);
  }
  if (res.status === 204) return null;
  if (!isJson) {
    const text = await res.text();
    if (text.trimStart().startsWith('<')) {
      throw new Error('Server returned HTML instead of JSON. Is the MeshMind API running?');
    }
    throw new Error('Invalid response: expected JSON');
  }
  return res.json();
}

export function setAdminToken(token) { adminToken = token; }

export const api = {
  getStatus: () => request('GET', '/status'),
  getPeers: () => request('GET', '/peers'),
  search: (q, limit = 20) => request('GET', `/search?q=${encodeURIComponent(q)}&limit=${limit}`),
  ask: (question, maxTokens = 1024) => request('POST', '/ask', { question, max_tokens: maxTokens }),
  getSources: () => request('GET', '/admin/sources'),
  approveSource: (sourceId, allowedTables = [], rowLimit = 0) =>
    request('POST', '/admin/sources/approve', { source_id: sourceId, allowed_tables: allowedTables, row_limit: rowLimit }),
  removeSource: (sourceId) =>
    request('POST', '/admin/sources/remove', { source_id: sourceId }),
  train: (target, datasetPreset) =>
    request('POST', '/admin/train', { target, dataset_preset: datasetPreset }),
  getModels: () => request('GET', '/admin/models'),
  rollbackModel: (modelId, fromVersion, toVersion, reason) =>
    request('POST', '/admin/models/rollback', { model_id: modelId, from_version: fromVersion, to_version: toVersion, reason }),
  getDatasets: () => request('GET', '/admin/datasets'),
  getFederatedStatus: () => request('GET', '/admin/federated/status'),
  getLogs: (n = 50) => request('GET', `/admin/logs?n=${n}`),
  scanSources: () => request('POST', '/admin/scan'),
  getScanDirs: () => request('GET', '/admin/scan-dirs'),
  addScanDir: (path) => request('POST', '/admin/scan-dirs/add', { path }),
  removeScanDir: (path) => request('POST', '/admin/scan-dirs/remove', { path }),
  ingestSource: (sourceId) => request('POST', '/admin/ingest', { source_id: sourceId }),
  approveAll: () => request('POST', '/admin/sources/approve-all'),
  ingestAll: () => request('POST', '/admin/ingest-all'),
  submitEvent: (eventId, title, summary, tags = []) =>
    request('POST', '/admin/event', { event_id: eventId, event_type: 'case_created', title, summary, tags }),
  listConversations: () => request('GET', '/conversations'),
  createConversation: () => request('POST', '/conversations'),
  getMessages: (conversationId) => request('GET', `/conversations/${encodeURIComponent(conversationId)}/messages`),
  sendMessage: (conversationId, content, maxTokens = 1024) =>
    request('POST', `/conversations/${encodeURIComponent(conversationId)}/messages`, { content, max_tokens: maxTokens }),
  /**
   * Send a message and stream the assistant response via SSE.
   * onToken(token) is called for each token; onDone(message) is called with the full message when done.
   * Returns a Promise that resolves when the stream ends or rejects on error.
   */
  async sendMessageStream(conversationId, content, maxTokens, { onToken, onDone }) {
    const opts = {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ content, max_tokens: maxTokens ?? 1024 }),
    };
    if (adminToken) opts.headers['Authorization'] = `Bearer ${adminToken}`;
    const res = await fetch(`${baseUrl}${API_PREFIX}/conversations/${encodeURIComponent(conversationId)}/messages/stream`, opts);
    if (!res.ok) {
      const err = await res.json().catch(() => ({}));
      throw new Error(err.error || `${res.status} ${res.statusText}`);
    }
    const reader = res.body.getReader();
    const decoder = new TextDecoder();
    let buf = '';
    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      buf += decoder.decode(value, { stream: true });
      const lines = buf.split('\n');
      buf = lines.pop() || '';
      for (const line of lines) {
        if (line.startsWith('data: ')) {
          try {
            const data = JSON.parse(line.slice(6));
            if (data.token != null && onToken) onToken(data.token);
            if (data.done && data.content != null && onDone) onDone(data);
          } catch (_) {}
        }
      }
    }
    if (buf.startsWith('data: ')) {
      try {
        const data = JSON.parse(buf.slice(6));
        if (data.done && data.content != null && onDone) onDone(data);
      } catch (_) {}
    }
  },
  deleteConversation: (conversationId) => request('DELETE', `/conversations/${encodeURIComponent(conversationId)}`),
  getConfigOnedrive: () => request('GET', '/admin/config/onedrive'),
  saveConfigOnedrive: (cfg) => request('POST', '/admin/config/onedrive', cfg),
  getOAuthStart: () => request('GET', '/admin/config/onedrive/oauth/start'),
  getConfigGeneral: () => request('GET', '/admin/config/general'),
  saveConfigGeneral: (cfg) => request('POST', '/admin/config/general', cfg),
  restart: () => request('POST', '/admin/restart'),
};
