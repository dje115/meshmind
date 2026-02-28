import { api } from './api.js';

let currentPage = 'dashboard';
let statusData = null;
let pollInterval = null;

// --- Toast system ---
function toast(message, type = 'info') {
  const container = document.getElementById('toast-container');
  const el = document.createElement('div');
  el.className = `toast toast-${type}`;
  el.textContent = message;
  container.appendChild(el);
  setTimeout(() => el.remove(), 3000);
}

// --- Navigation ---
document.querySelectorAll('.nav-item').forEach(item => {
  item.addEventListener('click', () => {
    const page = item.dataset.page;
    navigateTo(page);
  });
});

function navigateTo(page) {
  currentPage = page;
  document.querySelectorAll('.nav-item').forEach(i => i.classList.remove('active'));
  document.querySelector(`.nav-item[data-page="${page}"]`)?.classList.add('active');
  renderPage(page);
}

// --- Render pages ---
function renderPage(page) {
  const content = document.getElementById('content');
  switch (page) {
    case 'dashboard': renderDashboard(content); break;
    case 'ask': renderAsk(content); break;
    case 'sources': renderSources(content); break;
    case 'datasets': renderDatasets(content); break;
    case 'models': renderModels(content); break;
    case 'peers': renderPeers(content); break;
    case 'audit': renderAudit(content); break;
    default: content.innerHTML = '<div class="empty-state"><div class="empty-state-icon">?</div><div class="empty-state-text">Page not found</div></div>';
  }
}

// --- Dashboard ---
async function renderDashboard(el) {
  el.innerHTML = `
    <div class="page-header">
      <h1>Dashboard</h1>
      <p>System overview and quick actions</p>
    </div>
    <div class="stats-grid" id="dash-stats">
      <div class="stat-card"><div class="stat-label">Status</div><div class="stat-value"><div class="spinner"></div></div></div>
    </div>
    <div class="card">
      <div class="card-header">
        <span class="card-title">Quick Actions</span>
      </div>
      <div class="actions-row">
        <button class="btn btn-primary" onclick="window._nav('ask')">Ask a Question</button>
        <button class="btn btn-secondary" id="btn-train-quick">Train Now</button>
        <button class="btn btn-secondary" id="btn-scan-sources">Scan Sources</button>
      </div>
    </div>
    <div class="card">
      <div class="card-header"><span class="card-title">Recent Activity</span></div>
      <div id="dash-activity"><div class="empty-state"><div class="spinner"></div></div></div>
    </div>
  `;

  window._nav = navigateTo;

  document.getElementById('btn-train-quick')?.addEventListener('click', () => showTrainModal());
  document.getElementById('btn-scan-sources')?.addEventListener('click', async () => {
    const btn = document.getElementById('btn-scan-sources');
    btn.disabled = true;
    btn.textContent = 'Scanning...';
    toast('Source scan initiated', 'info');
    try {
      const result = await api.scanSources();
      toast(`Scan complete: ${result.sources_found} source(s) found`, 'success');
      renderDashboard(document.getElementById('content'));
    } catch (e) {
      toast(`Scan failed: ${e.message}`, 'error');
    }
    btn.disabled = false;
    btn.textContent = 'Scan Sources';
  });

  try {
    const [status, sources, models, datasets, logs] = await Promise.all([
      api.getStatus(),
      api.getSources().catch(() => []),
      api.getModels().catch(() => []),
      api.getDatasets().catch(() => []),
      api.getLogs(10).catch(() => []),
    ]);
    statusData = status;
    updateNodeStatus(true, status);

    document.getElementById('dash-stats').innerHTML = `
      <div class="stat-card">
        <div class="stat-label">Node Status</div>
        <div class="stat-value" style="color:var(--green)">Online</div>
        <div class="stat-sub">${status.backend} backend</div>
      </div>
      <div class="stat-card">
        <div class="stat-label">Events</div>
        <div class="stat-value">${status.event_count}</div>
        <div class="stat-sub">Total events logged</div>
      </div>
      <div class="stat-card">
        <div class="stat-label">Peers</div>
        <div class="stat-value">${status.peer_count}</div>
        <div class="stat-sub">Connected nodes</div>
      </div>
      <div class="stat-card">
        <div class="stat-label">Sources</div>
        <div class="stat-value">${sources.length}</div>
        <div class="stat-sub">Data sources discovered</div>
      </div>
      <div class="stat-card">
        <div class="stat-label">Models</div>
        <div class="stat-value">${models.length}</div>
        <div class="stat-sub">Trained models</div>
      </div>
      <div class="stat-card">
        <div class="stat-label">Datasets</div>
        <div class="stat-value">${datasets.length}</div>
        <div class="stat-sub">Built manifests</div>
      </div>
    `;

    const actEl = document.getElementById('dash-activity');
    if (logs.length === 0) {
      actEl.innerHTML = '<div class="empty-state"><div class="empty-state-text">No recent activity</div></div>';
    } else {
      actEl.innerHTML = `
        <div class="table-container"><table>
          <thead><tr><th>Event</th><th>Type</th><th>Summary</th><th>Time</th></tr></thead>
          <tbody>${logs.map(l => `
            <tr>
              <td style="font-family:var(--font-mono);font-size:12px">${l.event_id.slice(0, 16)}...</td>
              <td><span class="badge badge-default">${l.event_type}</span></td>
              <td>${escapeHtml(l.summary).slice(0, 80)}</td>
              <td style="color:var(--text-muted)">${formatTime(l.created_at_ms)}</td>
            </tr>
          `).join('')}</tbody>
        </table></div>
      `;
    }
  } catch (e) {
    updateNodeStatus(false);
    document.getElementById('dash-stats').innerHTML = `
      <div class="stat-card">
        <div class="stat-label">Status</div>
        <div class="stat-value" style="color:var(--red)">Offline</div>
        <div class="stat-sub">Cannot reach node API</div>
      </div>
    `;
    document.getElementById('dash-activity').innerHTML = '<div class="empty-state"><div class="empty-state-text">Node is not running</div></div>';
  }
}

// --- Ask / Chat ---
let chatState = { conversationId: null, conversations: [], sending: false };

async function renderAsk(el) {
  el.innerHTML = `
    <div class="chat-layout">
      <div class="chat-sidebar">
        <button class="btn btn-primary chat-new-btn" id="chat-new">+ New Chat</button>
        <div class="chat-conv-list" id="chat-conv-list"></div>
      </div>
      <div class="chat-main">
        <div class="chat-messages" id="chat-messages">
          <div class="chat-welcome">
            <div class="chat-welcome-icon">&#9670;</div>
            <h2>Ask MeshMind</h2>
            <p>Your local-first AI assistant. Ask about your documents, invoices, photos, or any ingested data.</p>
            <div class="chat-suggestions">
              <button class="chat-suggestion" data-q="What invoices do I have?">What invoices do I have?</button>
              <button class="chat-suggestion" data-q="Summarize my documents">Summarize my documents</button>
              <button class="chat-suggestion" data-q="What photos have GPS data?">What photos have GPS data?</button>
            </div>
          </div>
        </div>
        <div class="chat-input-bar">
          <textarea id="chat-input" placeholder="Message MeshMind..." rows="1"></textarea>
          <button class="btn btn-primary chat-send-btn" id="chat-send">&#9654;</button>
        </div>
      </div>
    </div>
  `;

  await loadConversationList();

  document.getElementById('chat-new').addEventListener('click', startNewChat);

  document.querySelectorAll('.chat-suggestion').forEach(btn => {
    btn.addEventListener('click', () => {
      document.getElementById('chat-input').value = btn.dataset.q;
      sendChatMessage();
    });
  });

  const textarea = document.getElementById('chat-input');
  textarea.addEventListener('keydown', e => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      sendChatMessage();
    }
  });
  textarea.addEventListener('input', () => {
    textarea.style.height = 'auto';
    textarea.style.height = Math.min(textarea.scrollHeight, 150) + 'px';
  });

  document.getElementById('chat-send').addEventListener('click', sendChatMessage);
}

async function loadConversationList() {
  try {
    chatState.conversations = await api.listConversations();
  } catch { chatState.conversations = []; }

  const list = document.getElementById('chat-conv-list');
  if (!list) return;

  if (chatState.conversations.length === 0) {
    list.innerHTML = '<div class="chat-conv-empty">No conversations yet</div>';
    return;
  }

  list.innerHTML = chatState.conversations.map(c => `
    <div class="chat-conv-item ${c.conversation_id === chatState.conversationId ? 'active' : ''}" data-id="${escapeHtml(c.conversation_id)}">
      <span class="chat-conv-title">${escapeHtml(c.title)}</span>
      <span class="chat-conv-time">${formatTimeShort(c.updated_at_ms)}</span>
      <button class="chat-conv-delete" data-id="${escapeHtml(c.conversation_id)}" title="Delete">&times;</button>
    </div>
  `).join('');

  list.querySelectorAll('.chat-conv-item').forEach(item => {
    item.addEventListener('click', (e) => {
      if (e.target.classList.contains('chat-conv-delete')) return;
      loadConversation(item.dataset.id);
    });
  });

  list.querySelectorAll('.chat-conv-delete').forEach(btn => {
    btn.addEventListener('click', async (e) => {
      e.stopPropagation();
      const id = btn.dataset.id;
      try {
        await api.deleteConversation(id);
        if (chatState.conversationId === id) {
          chatState.conversationId = null;
          showWelcome();
        }
        await loadConversationList();
      } catch (err) { toast(`Delete failed: ${err.message}`, 'error'); }
    });
  });
}

function showWelcome() {
  const area = document.getElementById('chat-messages');
  if (!area) return;
  area.innerHTML = `
    <div class="chat-welcome">
      <div class="chat-welcome-icon">&#9670;</div>
      <h2>Ask MeshMind</h2>
      <p>Your local-first AI assistant. Ask about your documents, invoices, photos, or any ingested data.</p>
      <div class="chat-suggestions">
        <button class="chat-suggestion" data-q="What invoices do I have?">What invoices do I have?</button>
        <button class="chat-suggestion" data-q="Summarize my documents">Summarize my documents</button>
        <button class="chat-suggestion" data-q="What photos have GPS data?">What photos have GPS data?</button>
      </div>
    </div>
  `;
  area.querySelectorAll('.chat-suggestion').forEach(btn => {
    btn.addEventListener('click', () => {
      document.getElementById('chat-input').value = btn.dataset.q;
      sendChatMessage();
    });
  });
}

async function loadConversation(convId) {
  chatState.conversationId = convId;
  await loadConversationList();

  const area = document.getElementById('chat-messages');
  if (!area) return;

  try {
    const msgs = await api.getMessages(convId);
    area.innerHTML = '';
    for (const m of msgs) {
      appendMessageBubble(m.role, m.content, m);
    }
    area.scrollTop = area.scrollHeight;
  } catch (e) {
    area.innerHTML = `<div class="empty-state"><div class="empty-state-text" style="color:var(--red)">Failed to load messages</div></div>`;
  }
}

async function startNewChat() {
  try {
    const conv = await api.createConversation();
    chatState.conversationId = conv.conversation_id;
    showWelcome();
    await loadConversationList();
    document.getElementById('chat-input')?.focus();
  } catch (e) { toast(`Error: ${e.message}`, 'error'); }
}

async function sendChatMessage() {
  if (chatState.sending) return;
  const textarea = document.getElementById('chat-input');
  const content = textarea.value.trim();
  if (!content) return;

  if (!chatState.conversationId) {
    try {
      const conv = await api.createConversation();
      chatState.conversationId = conv.conversation_id;
    } catch (e) { toast(`Error: ${e.message}`, 'error'); return; }
  }

  const area = document.getElementById('chat-messages');
  const welcome = area.querySelector('.chat-welcome');
  if (welcome) welcome.remove();

  appendMessageBubble('user', content);
  textarea.value = '';
  textarea.style.height = 'auto';

  const typingEl = document.createElement('div');
  typingEl.className = 'chat-bubble chat-bubble-assistant chat-typing';
  typingEl.innerHTML = '<div class="typing-indicator"><span></span><span></span><span></span></div>';
  area.appendChild(typingEl);
  area.scrollTop = area.scrollHeight;

  chatState.sending = true;
  const sendBtn = document.getElementById('chat-send');
  if (sendBtn) sendBtn.disabled = true;

  try {
    const resp = await api.sendMessage(chatState.conversationId, content);
    typingEl.remove();
    appendMessageBubble('assistant', resp.content, resp);
    area.scrollTop = area.scrollHeight;
    await loadConversationList();
  } catch (e) {
    typingEl.remove();
    appendMessageBubble('assistant', `Error: ${e.message}`, { confidence: 0, model: '', context_used: [] });
  }

  chatState.sending = false;
  if (sendBtn) sendBtn.disabled = false;
  textarea.focus();
}

function appendMessageBubble(role, content, meta) {
  const area = document.getElementById('chat-messages');
  if (!area) return;

  const bubble = document.createElement('div');
  bubble.className = `chat-bubble chat-bubble-${role}`;

  const textDiv = document.createElement('div');
  textDiv.className = 'chat-bubble-text';
  textDiv.textContent = content;
  bubble.appendChild(textDiv);

  if (role === 'assistant' && meta) {
    const metaDiv = document.createElement('div');
    metaDiv.className = 'chat-bubble-meta';
    const parts = [];
    if (meta.model) parts.push(meta.model);
    if (meta.confidence) parts.push(`${(meta.confidence * 100).toFixed(0)}%`);
    if (meta.context_used && meta.context_used.length) parts.push(`${meta.context_used.length} sources`);
    metaDiv.textContent = parts.join(' · ');
    bubble.appendChild(metaDiv);
  }

  area.appendChild(bubble);
}

function formatTimeShort(ms) {
  if (!ms) return '';
  const d = new Date(ms);
  const now = new Date();
  if (d.toDateString() === now.toDateString()) {
    return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  }
  return d.toLocaleDateString([], { month: 'short', day: 'numeric' });
}

// --- Sources ---
async function renderSources(el) {
  el.innerHTML = `
    <div class="page-header">
      <h1>Data Sources</h1>
      <p>Discovered data sources and their approval status</p>
    </div>
    <div id="sources-content"><div class="empty-state"><div class="spinner"></div></div></div>
  `;

  try {
    const sources = await api.getSources();
    const container = document.getElementById('sources-content');
    if (sources.length === 0) {
      container.innerHTML = `<div class="empty-state"><div class="empty-state-icon">⊟</div><div class="empty-state-text">No data sources discovered yet.<br/>Sources will appear here after running a discovery scan.</div></div>`;
      return;
    }
    container.innerHTML = `
      <div class="card"><div class="table-container"><table>
        <thead><tr><th>Source</th><th>Name</th><th>Type</th><th>Status</th><th>PII</th><th>Size</th><th>Actions</th></tr></thead>
        <tbody>${sources.map(s => `
          <tr>
            <td style="font-family:var(--font-mono);font-size:12px">${escapeHtml(s.source_id)}</td>
            <td>${escapeHtml(s.display_name)}</td>
            <td><span class="badge badge-blue">${connectorLabel(s.connector_type)}</span></td>
            <td>${statusBadge(s.status)}</td>
            <td>${s.pii_detected ? '<span class="badge badge-red">PII</span>' : '<span class="badge badge-green">Clean</span>'}</td>
            <td>${formatBytes(s.estimated_size_bytes)}</td>
            <td>${s.status !== 'approved'
              ? `<button class="btn btn-sm btn-primary" onclick="window._approveSource('${escapeHtml(s.source_id)}')">Approve</button>`
              : `<button class="btn btn-sm btn-primary" onclick="window._ingestSource('${escapeHtml(s.source_id)}')">Ingest</button>`}</td>
          </tr>
        `).join('')}</tbody>
      </table></div></div>
    `;

    window._approveSource = async (id) => {
      try {
        await api.approveSource(id);
        toast('Source approved', 'success');
        renderSources(el);
      } catch (e) {
        toast(`Error: ${e.message}`, 'error');
      }
    };

    window._ingestSource = async (id) => {
      const btn = document.querySelector(`button[onclick*="${id}"]`);
      if (btn) { btn.disabled = true; btn.textContent = 'Ingesting...'; }
      toast('Ingestion started...', 'info');
      try {
        const result = await api.ingestSource(id);
        toast(`Ingested: ${result.rows_ingested} rows, ${result.documents_created} docs, ${formatBytes(result.bytes_stored)} in ${result.duration_ms}ms`, 'success');
        renderSources(el);
      } catch (e) {
        toast(`Ingest error: ${e.message}`, 'error');
        if (btn) { btn.disabled = false; btn.textContent = 'Ingest'; }
      }
    };
  } catch (e) {
    document.getElementById('sources-content').innerHTML = `<div class="empty-state"><div class="empty-state-text" style="color:var(--red)">Failed to load sources: ${escapeHtml(e.message)}</div></div>`;
  }
}

// --- Datasets ---
async function renderDatasets(el) {
  el.innerHTML = `
    <div class="page-header">
      <h1>Datasets</h1>
      <p>Training dataset manifests for reproducible ML</p>
    </div>
    <div id="datasets-content"><div class="empty-state"><div class="spinner"></div></div></div>
  `;

  try {
    const datasets = await api.getDatasets();
    const container = document.getElementById('datasets-content');
    if (datasets.length === 0) {
      container.innerHTML = `<div class="empty-state"><div class="empty-state-icon">≡</div><div class="empty-state-text">No datasets built yet.<br/>Build a dataset from the Dashboard or use the Train action.</div></div>`;
      return;
    }
    container.innerHTML = `
      <div class="card"><div class="table-container"><table>
        <thead><tr><th>Manifest</th><th>Source</th><th>Preset</th><th>Items</th><th>Size</th></tr></thead>
        <tbody>${datasets.map(d => `
          <tr>
            <td style="font-family:var(--font-mono);font-size:12px">${escapeHtml(d.manifest_id).slice(0, 24)}...</td>
            <td>${escapeHtml(d.source_id || 'N/A')}</td>
            <td><span class="badge badge-blue">${escapeHtml(d.preset || 'custom')}</span></td>
            <td>${d.item_count}</td>
            <td>${formatBytes(d.total_bytes)}</td>
          </tr>
        `).join('')}</tbody>
      </table></div></div>
    `;
  } catch (e) {
    document.getElementById('datasets-content').innerHTML = `<div class="empty-state"><div class="empty-state-text" style="color:var(--red)">Failed to load datasets</div></div>`;
  }
}

// --- Models ---
async function renderModels(el) {
  el.innerHTML = `
    <div class="page-header">
      <h1>Models</h1>
      <p>Trained model registry with promotion and rollback controls</p>
    </div>
    <div id="models-content"><div class="empty-state"><div class="spinner"></div></div></div>
  `;

  try {
    const models = await api.getModels();
    const container = document.getElementById('models-content');
    if (models.length === 0) {
      container.innerHTML = `<div class="empty-state"><div class="empty-state-icon">◎</div><div class="empty-state-text">No models trained yet.<br/>Use the "Train Now" action to start training.</div></div>`;
      return;
    }
    container.innerHTML = `
      <div class="card"><div class="table-container"><table>
        <thead><tr><th>Model</th><th>Version</th><th>Status</th><th>Actions</th></tr></thead>
        <tbody>${models.map(m => `
          <tr>
            <td style="font-family:var(--font-mono);font-size:12px">${escapeHtml(m.model_id)}</td>
            <td>v${m.version}</td>
            <td>${m.promoted ? '<span class="badge badge-green">Promoted</span>' : m.rolled_back ? '<span class="badge badge-red">Rolled Back</span>' : '<span class="badge badge-default">Pending</span>'}</td>
            <td>${!m.rolled_back ? `<button class="btn btn-sm btn-danger" onclick="window._rollbackModel('${escapeHtml(m.model_id)}', ${m.version})">Rollback</button>` : ''}</td>
          </tr>
        `).join('')}</tbody>
      </table></div></div>
    `;

    window._rollbackModel = async (id, ver) => {
      try {
        await api.rollbackModel(id, ver, Math.max(1, ver - 1), 'Manual rollback from UI');
        toast('Model rolled back', 'success');
        renderModels(el);
      } catch (e) {
        toast(`Error: ${e.message}`, 'error');
      }
    };
  } catch (e) {
    document.getElementById('models-content').innerHTML = `<div class="empty-state"><div class="empty-state-text" style="color:var(--red)">Failed to load models</div></div>`;
  }
}

// --- Peers ---
async function renderPeers(el) {
  el.innerHTML = `
    <div class="page-header">
      <h1>Peers</h1>
      <p>Connected mesh nodes and their status</p>
    </div>
    <div id="peers-content"><div class="empty-state"><div class="spinner"></div></div></div>
  `;

  try {
    const peers = await api.getPeers();
    const container = document.getElementById('peers-content');
    if (peers.length === 0) {
      container.innerHTML = `<div class="empty-state"><div class="empty-state-icon">◇</div><div class="empty-state-text">No peers connected.<br/>Other MeshMind nodes on the LAN will appear here.</div></div>`;
      return;
    }
    container.innerHTML = `
      <div class="peer-grid">${peers.map(p => `
        <div class="peer-card">
          <div class="peer-name">${escapeHtml(p.node_id)}</div>
          <div class="peer-detail">Address: ${escapeHtml(p.address)}:${p.port}</div>
          <div class="peer-detail">State: ${statusBadge(p.state)}</div>
          <div class="peer-detail">RTT: ${p.rtt_ms ? `${p.rtt_ms}ms` : 'N/A'}</div>
          <div class="peer-detail">Capabilities: ${p.capabilities.length ? p.capabilities.map(c => `<span class="badge badge-blue">${escapeHtml(c)}</span>`).join(' ') : 'None'}</div>
        </div>
      `).join('')}</div>
    `;
  } catch (e) {
    document.getElementById('peers-content').innerHTML = `<div class="empty-state"><div class="empty-state-text" style="color:var(--red)">Failed to load peers</div></div>`;
  }
}

// --- Audit ---
async function renderAudit(el) {
  el.innerHTML = `
    <div class="page-header">
      <h1>Audit Log</h1>
      <p>Full event history for compliance and debugging</p>
    </div>
    <div id="audit-content"><div class="empty-state"><div class="spinner"></div></div></div>
  `;

  try {
    const logs = await api.getLogs(100);
    const container = document.getElementById('audit-content');
    if (logs.length === 0) {
      container.innerHTML = `<div class="empty-state"><div class="empty-state-icon">◈</div><div class="empty-state-text">No audit entries yet</div></div>`;
      return;
    }
    container.innerHTML = `
      <div class="card"><div class="table-container"><table>
        <thead><tr><th>Event ID</th><th>Type</th><th>Summary</th><th>Time</th></tr></thead>
        <tbody>${logs.map(l => `
          <tr>
            <td style="font-family:var(--font-mono);font-size:12px">${escapeHtml(l.event_id)}</td>
            <td><span class="badge badge-default">${l.event_type}</span></td>
            <td>${escapeHtml(l.summary).slice(0, 100)}</td>
            <td style="color:var(--text-muted);white-space:nowrap">${formatTime(l.created_at_ms)}</td>
          </tr>
        `).join('')}</tbody>
      </table></div></div>
    `;
  } catch (e) {
    document.getElementById('audit-content').innerHTML = `<div class="empty-state"><div class="empty-state-text" style="color:var(--red)">Failed to load audit log</div></div>`;
  }
}

// --- Train modal ---
function showTrainModal() {
  const overlay = document.createElement('div');
  overlay.className = 'modal-overlay';
  overlay.innerHTML = `
    <div class="modal">
      <div class="modal-header">
        <span class="modal-title">Train Model</span>
        <button class="modal-close" id="modal-close">&times;</button>
      </div>
      <div class="form-group">
        <label class="form-label">Target</label>
        <select id="train-target">
          <option value="router">Router / Classifier</option>
          <option value="tagger">Tagger / Classifier</option>
          <option value="ranker">Ranker (Experimental)</option>
        </select>
      </div>
      <div class="form-group">
        <label class="form-label">Dataset Preset</label>
        <select id="train-preset">
          <option value="public_shareable_only">Public Shareable Only</option>
          <option value="this_tenant_confirmed">This Tenant Confirmed</option>
          <option value="all_approved_no_restricted">All Approved (No Restricted)</option>
          <option value="numeric_only">Numeric Only (Anomaly)</option>
        </select>
      </div>
      <div class="modal-footer">
        <button class="btn btn-secondary" id="modal-cancel">Cancel</button>
        <button class="btn btn-primary" id="modal-train">Start Training</button>
      </div>
    </div>
  `;
  document.body.appendChild(overlay);

  overlay.querySelector('#modal-close').addEventListener('click', () => overlay.remove());
  overlay.querySelector('#modal-cancel').addEventListener('click', () => overlay.remove());
  overlay.addEventListener('click', e => { if (e.target === overlay) overlay.remove(); });

  overlay.querySelector('#modal-train').addEventListener('click', async () => {
    const target = document.getElementById('train-target').value;
    const preset = document.getElementById('train-preset').value;
    try {
      const res = await api.train(target, preset);
      if (res.status === 'completed') {
        toast(`Training complete! Score: ${(res.score * 100).toFixed(1)}% | Model: ${res.model_version} | Dataset: ${res.dataset_items} items`, 'success');
      } else {
        toast(`Training ${res.status} (${res.dataset_items} dataset items)`, res.status.startsWith('rejected') || res.status.startsWith('failed') ? 'error' : 'info');
      }
      overlay.remove();
      renderPage(currentPage);
    } catch (e) {
      toast(`Error: ${e.message}`, 'error');
    }
  });
}

// --- Helpers ---
function updateNodeStatus(connected, data) {
  const dot = document.querySelector('.status-dot');
  const text = document.querySelector('.status-text');
  if (connected && data) {
    dot.className = 'status-dot connected';
    text.textContent = `${data.node_id.slice(0, 12)}...`;
  } else {
    dot.className = 'status-dot error';
    text.textContent = 'Offline';
  }
}

function escapeHtml(str) {
  if (!str) return '';
  return str.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

function formatTime(ms) {
  if (!ms) return 'N/A';
  const d = new Date(ms);
  return d.toLocaleString();
}

function formatBytes(bytes) {
  if (!bytes || bytes === 0) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB'];
  let i = 0;
  let val = bytes;
  while (val >= 1024 && i < units.length - 1) { val /= 1024; i++; }
  return `${val.toFixed(i > 0 ? 1 : 0)} ${units[i]}`;
}

function connectorLabel(type) {
  const labels = { 1: 'SQLite', 2: 'CSV', 3: 'JSON', 7: 'Images', 8: 'Documents' };
  return labels[type] || `Type ${type}`;
}

function statusBadge(status) {
  const map = {
    discovered: 'badge-blue',
    classified: 'badge-orange',
    approved: 'badge-green',
    started: 'badge-blue',
    completed: 'badge-green',
    failed: 'badge-red',
    reachable: 'badge-green',
    suspected: 'badge-orange',
    unreachable: 'badge-red',
  };
  return `<span class="badge ${map[status] || 'badge-default'}">${escapeHtml(status)}</span>`;
}

// --- Status polling ---
async function pollStatus() {
  try {
    const s = await api.getStatus();
    statusData = s;
    updateNodeStatus(true, s);
  } catch {
    updateNodeStatus(false);
  }
}

// --- Init ---
renderPage('dashboard');
pollInterval = setInterval(pollStatus, 10000);
pollStatus();
