const BASE = 'http://127.0.0.1:3000';

async function request(method, path, body) {
  const opts = { method, headers: {} };
  if (body) {
    opts.headers['Content-Type'] = 'application/json';
    opts.body = JSON.stringify(body);
  }
  const res = await fetch(`${BASE}${path}`, opts);
  if (!res.ok) throw new Error(`${res.status} ${res.statusText}`);
  return res.json();
}

export const api = {
  getStatus: () => request('GET', '/status'),
  getPeers: () => request('GET', '/peers'),
  search: (q, limit = 20) => request('GET', `/search?q=${encodeURIComponent(q)}&limit=${limit}`),
  ask: (question, maxTokens = 1024) => request('POST', '/ask', { question, max_tokens: maxTokens }),
  getSources: () => request('GET', '/admin/sources'),
  approveSource: (sourceId, allowedTables = [], rowLimit = 0) =>
    request('POST', '/admin/sources/approve', { source_id: sourceId, allowed_tables: allowedTables, row_limit: rowLimit }),
  train: (target, datasetPreset) =>
    request('POST', '/admin/train', { target, dataset_preset: datasetPreset }),
  getModels: () => request('GET', '/admin/models'),
  rollbackModel: (modelId, fromVersion, toVersion, reason) =>
    request('POST', '/admin/models/rollback', { model_id: modelId, from_version: fromVersion, to_version: toVersion, reason }),
  getDatasets: () => request('GET', '/admin/datasets'),
  getLogs: (n = 50) => request('GET', `/admin/logs?n=${n}`),
  submitEvent: (eventId, title, summary, tags = []) =>
    request('POST', '/admin/event', { event_id: eventId, event_type: 'case_created', title, summary, tags }),
};
