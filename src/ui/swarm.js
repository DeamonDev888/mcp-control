/* ===== Onglet Swarm Control (instances admin + agents MCP résidents) =====
   Dépend des helpers globaux de app.js : postAction, esc, escPre, ic, toast,
   askConfirm, load, state, selectedId, activeView, swarmLogoUrl.
   CSP stricte : aucun attribut style, aucune mutation directe de style, aucun on*, toutes les actions dans ACT. */

let swarmTab = 'hive'; // 'hive' | 'agents' | 'console' | 'runs'
let swarmData = null;
let swarmLoading = false;
let swarmLive = true;
let swarmA2aFilter = 'all'; // 'all' | 'a2a' | 'orch' | 'runs' | 'err'
let swarmA2aEvents = []; // [{ts, kind, from, to, via, meta, body, err}]
let swarmSelectedInst = '';
let swarmFilterRunner = 'all';
let swarmFilterState = 'all';
let swarmSearch = '';
let swarmCreateOpen = false;
let swarmNewClass = 'coder';
let swarmConsoleTarget = ''; // 'res:<server_id>' | 'inst:<instance_id>:<agent_name>'
let swarmFeedsByTarget = {}; // { [target]: [{ts, role, title, meta, text, err, fromJournal}] }
let swarmHistLoaded = {}; // { [server_id]: { loaded: true, oldestTs: u64|null, hasMore: bool } }
let swarmResLogsOpen = {}; // { [server_id]: { open: bool, output: string, live: bool, bytes: number } }
let swarmResidentLive = {}; // { [server_id]: statusObj }
let swarmCfgAgent = '';
let swarmCfgFile = 'prompt.md';
let swarmCfgData = null;
let swarmStreamOut = null; // {runId, agentName, text}
let swarmPollTimer = null;
let swarmParallelDraft = [
  {runner: 'claude', agentName: '', prompt: ''},
  {runner: 'claude', agentName: '', prompt: ''},
];

const SWARM_CLASSES = [
  ['coder', 'Coder (développement)'],
  ['coordinator', 'Coordinator (orchestration)'],
  ['explorer', 'Explorer (recherche / audit)'],
  ['verifier', 'Verifier (tests / revue)'],
  ['monitor', 'Monitor (supervision)'],
  ['browser', 'Browser (navigation web)'],
  ['discord', 'Discord'],
  ['google', 'Google Workspace'],
  ['whatsapp', 'WhatsApp'],
  ['x', 'X / Twitter'],
  ['custom', 'Custom (personnalisé)'],
];

function swarmFeedFor(target) {
  const k = target || '_default';
  if (!swarmFeedsByTarget[k]) swarmFeedsByTarget[k] = [];
  return swarmFeedsByTarget[k];
}

function swarmRerenderActive() {
  if (activeView === 'swarm') renderSwarmView();
  if (selectedId && detailCache && (detailTab === 'swarm-chat' || detailTab === 'swarm-cfg')) {
    _sigDetail = '';
    renderDetail();
  }
}

function swarmIconSvg(cls, name, kind, agentCls) {
  const src = swarmLogoUrl(name || '', kind || 'hive', agentCls);
  return `<img class="${cls || 'ic-hornet'} swarm-img" src="${esc(src)}" alt="" aria-hidden="true">`;
}

function swarmStopPoll() {
  if (swarmPollTimer) { clearInterval(swarmPollTimer); swarmPollTimer = null; }
}

function swarmTypingInView() {
  const el = document.activeElement;
  if (!el || !el.id) return false;
  return [
    'swA2aPrompt', 'swA2aFrom', 'swA2aVia', 'swAskPrompt', 'swAskFrom', 'swAskModel', 'swAskPath',
    'swCfgEditor', 'swNewPrompt', 'swNewName', 'swNewModel', 'swNewClone', 'swAgentSearch',
  ].includes(el.id) || (el.classList && (el.classList.contains('sw-par-prompt') || el.classList.contains('sw-par-agent')));
}

function swarmEnsurePoll() {
  if (swarmPollTimer) return;
  swarmPollTimer = setInterval(() => {
    const inDetailSwarm = !!(selectedId && (detailTab === 'swarm-chat' || detailTab === 'swarm-cfg'));
    if (activeView !== 'swarm' && !inDetailSwarm) {
      swarmStopPoll();
      return;
    }
    const hasRunning = (swarmData && swarmData.jobs || []).some(j => j.status === 'running');
    if (hasRunning) swarmPollJobs();
    // Si un tiroir logs résident est ouvert, rafraîchir sa sortie live
    for (const sid of Object.keys(swarmResLogsOpen)) {
      if (swarmResLogsOpen[sid] && swarmResLogsOpen[sid].open) {
        swarmFetchResLogs(sid, true);
      }
    }
    if (activeView === 'swarm' && swarmLive && !swarmLoading && !swarmTypingInView()) {
      swarmRefresh(false);
    }
  }, 5000);
}

function openSwarmView(subTab, focusTarget) {
  closeDiscoverMenu();
  ctxClose(false);
  selectedId = null;
  detailCache = null;
  activeView = 'swarm';
  explorerTab = 'swarm';
  if (subTab) swarmTab = subTab === 'overview' ? 'hive' : subTab;
  if (focusTarget) swarmConsoleTarget = focusTarget;
  updateSwarmTopbarActive();
  renderServers();
  renderSwarmView();
  swarmRefresh(false).then(() => {
    if (swarmConsoleTarget.startsWith('res:')) {
      swarmEnsureResidentHistory(swarmConsoleTarget.slice(4));
    }
  });
  swarmEnsurePoll();
}

function updateSwarmTopbarActive() {
  const bMcp = document.getElementById('explTabMcp');
  const bSw = document.getElementById('explTabSwarm');
  const on = explorerTab === 'swarm' || activeView === 'swarm';
  if (bMcp) { bMcp.classList.toggle('sel', !on); bMcp.setAttribute('aria-selected', !on ? 'true' : 'false'); }
  if (bSw) { bSw.classList.toggle('sel', on); bSw.setAttribute('aria-selected', on ? 'true' : 'false'); }
}

function updateSwarmBadge() {
  const el = document.getElementById('tbSwarmN');
  if (!el || !state) return;
  const st = state.swarm_stats || {instances: 0, residents: 0, up: 0};
  const tot = st.residents || 0;
  if (tot > 0) {
    el.textContent = String(tot);
    el.hidden = false;
  } else {
    el.hidden = true;
  }
  const tabImg = document.querySelector('#explTabSwarm .ic-hornet');
  if (tabImg) {
    const want = swarmLogoUrl('logo', 'hive');
    if (tabImg.getAttribute('src') !== want) tabImg.setAttribute('src', want);
  }
}

async function swarmRefresh(showToast) {
  if (swarmLoading) return;
  swarmLoading = true;
  if (activeView === 'swarm' && !swarmData) renderSwarmView();
  try {
    const j = await postAction({action: 'swarm_overview'}, {quiet: !showToast});
    if (j && j.ok) {
      swarmData = j;
      if (state && Array.isArray(state.servers) && Array.isArray(j.residents)) {
        let clsChanged = false;
        for (const r of j.residents) {
          const srv = state.servers.find(s => s.id === r.id);
          const nextCls = r.swarm_class || (r.status && r.status.class) || null;
          if (srv && (srv.swarm_class || null) !== nextCls) {
            srv.swarm_class = nextCls;
            clsChanged = true;
          }
        }
        if (clsChanged) {
          _sigServers = '';
          renderServers();
        }
      }
      if (!swarmSelectedInst || !(j.instances || []).some(i => i.id === swarmSelectedInst)) {
        if (j.instances && j.instances.length) {
          const upInst = j.instances.find(i => i.state === 'UP') || j.instances[0];
          swarmSelectedInst = upInst.id;
        }
      }
      if (!swarmConsoleTarget) {
        if (j.residents && j.residents.length) {
          const upRes = j.residents.find(r => r.state === 'UP') || j.residents[0];
          swarmConsoleTarget = 'res:' + upRes.id;
        } else {
          const allAg = swarmAllAgents();
          if (allAg.length) swarmConsoleTarget = 'inst:' + allAg[0]._instanceId + ':' + allAg[0].name;
        }
      }
      if (showToast) toast('Essaim Swarm actualis\u00e9', 'ok', 1800);
    }
  } finally {
    swarmLoading = false;
    swarmRerenderActive();
  }
}

/* Convertit les entrées du journal `history` de Swarm-control en messages UI. */
function swarmEntriesToMessages(entries, fallbackAgent) {
  const msgs = [];
  for (const e of (entries || [])) {
    const ts = e.ts || Date.now();
    const fromWho = e.from || 'utilisateur';
    const viaTxt = (Array.isArray(e.via) && e.via.length) ? (' \u00b7 via ' + e.via.join(' \u2192 ')) : '';
    const dateStr = e.ts ? new Date(e.ts).toLocaleTimeString('fr-FR', {hour: '2-digit', minute: '2-digit', second: '2-digit'}) : '';
    if (e.prompt) {
      msgs.push({
        ts,
        role: 'user',
        title: fromWho,
        meta: dateStr + viaTxt,
        text: String(e.prompt),
        err: false,
        fromJournal: true,
      });
    }
    const metaParts = [];
    if (dateStr) metaParts.push(dateStr);
    if (e.durationMs != null) metaParts.push(e.durationMs + ' ms');
    if (e.queuedMs != null && e.queuedMs > 0) metaParts.push('file ' + e.queuedMs + ' ms');
    if (e.sessionId) metaParts.push('session ' + String(e.sessionId).slice(0, 12));
    const hasErr = Boolean(e.error);
    const replyTxt = hasErr ? String(e.error) : (e.reply != null ? String(e.reply) : '');
    if (replyTxt || hasErr) {
      msgs.push({
        ts: ts + 1,
        role: 'agent',
        title: e.agent || fallbackAgent || 'agent',
        meta: metaParts.join(' \u00b7 '),
        text: replyTxt,
        err: hasErr,
        fromJournal: true,
      });
    }
  }
  return msgs;
}

async function swarmEnsureResidentHistory(serverId) {
  if (!serverId) return;
  const st = swarmHistLoaded[serverId];
  if (st && st.loaded) return;
  const srv = (state && state.servers || []).find(s => s.id === serverId);
  if (srv && srv.state !== 'UP') return;
  swarmHistLoaded[serverId] = {loaded: true, oldestTs: null, hasMore: false};
  const j = await postAction({action: 'swarm_resident_history', server_id: serverId, limit: 30}, {quiet: true});
  if (!j || !j.ok || !j.data) return;
  const entries = j.data.entries || [];
  if (!entries.length) return;
  const oldest = entries[0] && entries[0].ts ? entries[0].ts : null;
  swarmHistLoaded[serverId] = {
    loaded: true,
    oldestTs: oldest,
    hasMore: entries.length >= 30 && oldest != null,
  };
  const key = 'res:' + serverId;
  const existing = swarmFeedFor(key).filter(m => !m.fromJournal);
  const journalMsgs = swarmEntriesToMessages(entries, j.data.agent);
  swarmFeedsByTarget[key] = journalMsgs.concat(existing);
  swarmRerenderActive();
}

async function swarmLoadMoreHistory(serverId) {
  const info = swarmHistLoaded[serverId];
  if (!serverId || !info || !info.oldestTs) return;
  const j = await postAction({
    action: 'swarm_resident_history',
    server_id: serverId,
    limit: 30,
    before: info.oldestTs,
  });
  if (!j || !j.ok || !j.data) return;
  const entries = j.data.entries || [];
  const oldest = entries.length && entries[0].ts ? entries[0].ts : null;
  swarmHistLoaded[serverId] = {
    loaded: true,
    oldestTs: oldest,
    hasMore: entries.length >= 30 && oldest != null,
  };
  if (!entries.length) {
    toast('Aucun tour plus ancien', 'ok', 1800);
    swarmRerenderActive();
    return;
  }
  const key = 'res:' + serverId;
  const olderMsgs = swarmEntriesToMessages(entries, j.data.agent);
  swarmFeedsByTarget[key] = olderMsgs.concat(swarmFeedFor(key));
  swarmRerenderActive();
}

async function swarmFetchResLogs(serverId, quiet) {
  if (!serverId) return;
  const j = await postAction({action: 'swarm_resident_logs', server_id: serverId, tail: 16384}, {quiet: !!quiet});
  if (j && j.ok && j.data) {
    const cur = swarmResLogsOpen[serverId] || {open: true};
    swarmResLogsOpen[serverId] = {
      open: cur.open,
      output: j.data.output || '',
      live: !!j.data.live,
      bytes: j.data.bytes || 0,
    };
    swarmRerenderActive();
  }
}

async function swarmToggleResLogs(serverId) {
  if (!serverId) return;
  const cur = swarmResLogsOpen[serverId];
  if (cur && cur.open) {
    cur.open = false;
    swarmRerenderActive();
    return;
  }
  swarmResLogsOpen[serverId] = {open: true, output: 'Chargement des logs\u2026', live: false, bytes: 0};
  swarmRerenderActive();
  await swarmFetchResLogs(serverId, false);
}

async function swarmResSessionReset(serverId) {
  if (!serverId) return;
  const ok = await askConfirm({
    title: 'R\u00e9initialiser la session de l\'agent ?',
    msg: 'L\'identifiant de session courant sera oubli\u00e9.',
    detail: 'Le prochain tour repartira sur une nouvelle session vierge (l\'historique JSONL sur disque est conserv\u00e9).',
    okLabel: 'Nouvelle session',
  });
  if (!ok) return;
  const j = await postAction({action: 'swarm_resident_session', server_id: serverId, op: 'reset'});
  if (j && j.ok) {
    toast('Session r\u00e9initialis\u00e9e \u2014 le prochain tour repartira \u00e0 neuf', 'ok', 2800);
    await swarmResStatus(serverId);
  }
}

async function swarmResSessionSet(serverId) {
  if (!serverId) return;
  const res = await askConfirm({
    title: 'Forcer / reprendre une session',
    msg: 'Indiquez l\'identifiant exact de la session \u00e0 associer \u00e0 cet agent r\u00e9sident :',
    inputs: [{id: 'sid', label: 'Identifiant de session', placeholder: 'ex. sess-... ou UUID Claude/Hermes'}],
    okLabel: 'Appliquer',
  });
  const sid = res && typeof res === 'object' ? (res.sid || '').trim() : '';
  if (!sid) return;
  const j = await postAction({action: 'swarm_resident_session', server_id: serverId, op: 'set', session_id: sid});
  if (j && j.ok) {
    toast(`Session active : ${sid}`, 'ok', 2600);
    await swarmResStatus(serverId);
  }
}

async function swarmChangeAgentClass(el) {
  if (!el) return;
  const cls = el.value || '';
  const agent = el.dataset.agent || '';
  const serverId = el.dataset.id || '';
  const payload = {action: 'swarm_set_agent_class', class: cls};
  if (agent) payload.agent = agent;
  if (serverId) payload.server_id = serverId;
  const j = await postAction(payload);
  if (j && j.ok) {
    toast(`Classe mise \u00e0 jour : ${j.class || 'auto'}`, 'ok', 2000);
    await load();
    await swarmRefresh(false);
  }
}

async function swarmFollowResidentUrl(url, agentName) {
  if (!url) return;
  let port = 0;
  let path = '/mcp';
  try {
    const u = new URL(url);
    port = Number(u.port);
    path = u.pathname || '/mcp';
  } catch (_) {}
  if (!port) { toast('URL r\u00e9sident invalide', 'err', 2500); return; }
  const j = await postAction({
    action: 'add',
    label: 'swarm-' + (agentName || port),
    transport: 'http',
    port,
    path,
  });
  if (j && j.ok) {
    toast(`R\u00e9sident swarm-${agentName || port} suivi dans MCP Control`, 'ok', 2500);
    await load();
    await swarmRefresh(false);
  }
}

async function swarmPollJobs() {
  const j = await postAction({action: 'swarm_job_status'}, {quiet: true});
  if (!j || !j.ok || !swarmData) return;
  const prev = new Map((swarmData.jobs || []).map(x => [x.id, x]));
  swarmData.jobs = j.jobs || [];
  let completedAny = false;
  for (const job of swarmData.jobs) {
    const old = prev.get(job.id);
    if (old && old.status === 'running' && job.status !== 'running') {
      completedAny = true;
      const feedKey = job.kind === 'ask'
        ? ('res:' + job.server_id)
        : (job.kind === 'run_agent' && job.agent ? ('inst:' + job.server_id + ':' + job.agent) : swarmConsoleTarget);
      if (job.kind === 'ask' || job.kind === 'run_agent') {
        const feed = swarmFeedFor(feedKey);
        if (job.status === 'done' && job.result) {
          const r = job.result;
          const metaParts = [];
          if (r.durationMs != null) metaParts.push(r.durationMs + ' ms');
          if (r.queuedMs != null && r.queuedMs > 0) metaParts.push('file ' + r.queuedMs + ' ms');
          if (r.sessionId) metaParts.push('session ' + r.sessionId);
          if (r.via && r.via.length) metaParts.push('via ' + r.via.join(' \u2192 '));
          feed.push({
            ts: Date.now(),
            role: 'agent',
            title: r.agent || job.agent || job.server_id,
            meta: metaParts.join(' \u00b7 '),
            text: typeof r.reply === 'string' ? r.reply : (typeof r.output === 'string' ? r.output : JSON.stringify(r, null, 2)),
            err: false,
          });
        } else {
          feed.push({
            ts: Date.now(),
            role: 'agent',
            title: job.agent || job.server_id,
            meta: 'erreur',
            text: job.error || 'erreur inconnue',
            err: true,
          });
        }
      } else if (job.status === 'done') {
        toast(`T\u00e2che Swarm ${job.id} termin\u00e9e`, 'ok', 2500);
      } else {
        toast(`T\u00e2che Swarm ${job.id} : ${job.error || 'erreur'}`, 'err', 4000);
      }
    }
  }
  if (completedAny) {
    swarmRerenderActive();
    swarmRefresh(false);
  } else {
    swarmRerenderActive();
  }
}

function swarmSetTab(tab) {
  swarmTab = (!tab || tab === 'overview') ? 'hive' : tab;
  renderSwarmView();
  if (swarmTab === 'console' && swarmConsoleTarget.startsWith('res:')) {
    swarmEnsureResidentHistory(swarmConsoleTarget.slice(4));
  }
}

function swarmQuickCreate() {
  swarmTab = 'agents';
  swarmCreateOpen = true;
  renderSwarmView();
  const inp = document.getElementById('swNewName');
  if (inp) inp.focus();
}

function swarmNewClassChange(el) {
  swarmNewClass = (el && el.value) || 'coder';
  const prevImg = document.querySelector('#swNewClsPrev img');
  if (prevImg) prevImg.setAttribute('src', swarmLogoUrl(swarmNewClass, 'resident', swarmNewClass));
}

function swarmAllAgents() {
  const out = [];
  if (!swarmData) return out;
  for (const inst of (swarmData.instances || [])) {
    for (const a of (inst.agents || [])) {
      out.push(Object.assign({}, a, {
        _instanceId: inst.id,
        _instanceLabel: inst.label,
        _instanceState: inst.state,
      }));
    }
  }
  return out;
}

function swarmFindResidentFor(agentName) {
  if (!swarmData || !agentName) return null;
  const low = String(agentName).toLowerCase();
  return (swarmData.residents || []).find(r =>
    (r.agent && r.agent.toLowerCase() === low) ||
    (r.label && r.label.toLowerCase() === 'swarm-' + low)
  ) || null;
}

function swarmCaptureInputs(root) {
  const snap = {};
  if (!root) return snap;
  root.querySelectorAll('input[id], textarea[id], select[id]').forEach(el => {
    if (el.type === 'checkbox') snap[el.id] = {checked: el.checked};
    else snap[el.id] = {value: el.value};
  });
  return snap;
}

function swarmRestoreInputs(root, snap) {
  if (!root || !snap) return;
  for (const id of Object.keys(snap)) {
    const el = document.getElementById(id);
    if (!el) continue;
    if (id === 'swCfgEditor' || id === 'swConsoleTarget' || id === 'swCfgAgentSel') continue;
    const s = snap[id];
    if (el.type === 'checkbox' && typeof s.checked === 'boolean') el.checked = s.checked;
    else if (typeof s.value === 'string' && !el.value && s.value) el.value = s.value;
  }
}

function renderSwarmBanners(insts, agents, anns) {
  const parts = [];
  for (const inst of insts) {
    if (inst.locked && !inst.paired) {
      parts.push(`<div class="sw-banner is-warn">
        <div class="sw-banner-l">${ic('lock')} <div><b>${esc(inst.label)}</b> est verrouill&eacute; par signature Ed25519 (RFC 9421) &mdash; appairez-le pour lister et piloter ses agents.</div></div>
        <div class="sw-banner-acts">
          <button type="button" class="primary" data-act="server-pair" data-id="${esc(inst.id)}">${ic('lock')} Appairer</button>
          <button type="button" data-act="select" data-id="${esc(inst.id)}">Fiche serveur</button>
        </div>
      </div>`);
    } else if (inst.state !== 'UP') {
      parts.push(`<div class="sw-banner is-warn">
        <div class="sw-banner-l"><span class="dot ${esc(inst.state)}"></span> <div>L'orchestrateur <b>${esc(inst.label)}</b> est actuellement <b>${esc(inst.state)}</b>.</div></div>
        <div class="sw-banner-acts">
          <button type="button" class="primary" data-act="row-act" data-op="start" data-id="${esc(inst.id)}">${ic('play')} D&eacute;marrer</button>
          <button type="button" data-act="select" data-id="${esc(inst.id)}">Fiche serveur</button>
        </div>
      </div>`);
    } else if (inst.error) {
      parts.push(`<div class="sw-banner is-err">
        <div class="sw-banner-l">${ic('warn')} <div><b>${esc(inst.label)}</b> &mdash; erreur lors de l'interrogation MCP : <code>${esc(inst.error)}</code>${inst.stale_agents ? ' (liste en cache affich&eacute;e)' : ''}</div></div>
        <div class="sw-banner-acts">
          <button type="button" data-act="swarm-refresh">${ic('refresh')} R&eacute;essayer</button>
          <button type="button" data-act="select" data-id="${esc(inst.id)}">Fiche serveur</button>
        </div>
      </div>`);
    }
  }

  for (const a of anns) {
    parts.push(`<div class="sw-banner">
      <div class="sw-banner-l"><span class="tag swarm-res">D&Eacute;TECT&Eacute;</span> <div>Agent r&eacute;sident <b>${esc(a.name)}</b> actif sur le port <code>:${esc(String(a.port || '?'))}</code>${a.version ? ` (v${esc(a.version)})` : ''} mais non suivi.</div></div>
      <div class="sw-banner-acts">
        <button type="button" class="primary" data-act="follow-ann" data-name="${esc(a.name)}" data-port="${esc(String(a.port || ''))}">Suivre dans MCP Control</button>
      </div>
    </div>`);
  }

  const upHealthy = insts.filter(i => i.state === 'UP' && !i.error && !(i.locked && !i.paired));
  if (!insts.length && swarmData) {
    const storeSrv = (state && state.servers || []).find(s => s.id === 'store-swarm-control');
    if (!storeSrv) {
      parts.push(`<div class="sw-banner is-warn">
        <div class="sw-banner-l">${ic('store')} <div>Aucun orchestrateur <b>Swarm Control</b> n'est install&eacute; ou d&eacute;tect&eacute; sur les ports <code>23080&ndash;23099</code>.</div></div>
        <div class="sw-banner-acts">
          <button type="button" class="primary" data-act="open-store-update" data-store-id="swarm-control">${ic('download')} Installer depuis la Boutique</button>
          <button type="button" data-act="scan">${ic('search')} Scanner les ports</button>
        </div>
      </div>`);
    }
  } else if (upHealthy.length > 0 && agents.length === 0 && swarmTab !== 'agents') {
    parts.push(`<div class="sw-banner is-ok">
      <div class="sw-banner-l">${swarmIconSvg('ic-hornet', 'coordinator', 'resident', 'coordinator')} <div><b>${esc(upHealthy[0].label)}</b> est actif et pr&ecirc;t, mais aucun agent persistant n'est encore configur&eacute;.</div></div>
      <div class="sw-banner-acts">
        <button type="button" class="primary" data-act="swarm-quick-create">+ Cr&eacute;er mon premier agent</button>
      </div>
    </div>`);
  }
  return parts.join('');
}

function renderSwarmView() {
  const contentEl = document.getElementById('content');
  if (!contentEl || activeView !== 'swarm') return;
  const savedInputs = swarmCaptureInputs(contentEl);
  delete contentEl.dataset.srvId;

  const insts = (swarmData && swarmData.instances) || [];
  const ress = (swarmData && swarmData.residents) || [];
  const anns = ((swarmData && swarmData.announcements) || []).filter(a => !a.followed && a.swarm_kind === 'resident');
  const agents = swarmAllAgents();
  const jobs = (swarmData && swarmData.jobs) || [];
  const upInsts = insts.filter(i => i.state === 'UP');
  const resUp = ress.filter(r => r.state === 'UP').length;
  const runningJobs = jobs.filter(j => j.status === 'running').length;
  const activeRuns = insts.reduce((acc, i) => acc + ((i.runs || []).length), 0);
  const totalTurns = ress.reduce((acc, r) => acc + (((swarmResidentLive[r.id] || r.status || {}).turnsDone) || 0), 0);

  const storeInstalled = Boolean(insts.length || (state && (state.servers || []).some(s => s.id === 'store-swarm-control' || s.swarm_kind === 'instance')));
  const primaryInst = upInsts[0] || insts[0] || null;

  let primaryCta = '';
  if (!storeInstalled) {
    primaryCta = `<button type="button" class="primary" data-act="open-store-update" data-store-id="swarm-control">${ic('download')} Installer Swarm</button>`;
  } else if (primaryInst && primaryInst.state !== 'UP') {
    primaryCta = `<button type="button" class="primary" data-act="row-act" data-op="start" data-id="${esc(primaryInst.id)}">${ic('play')} D&eacute;marrer Swarm</button>`;
  } else {
    primaryCta = `<button type="button" class="primary" data-act="swarm-quick-create">+ Nouvel agent</button>`;
  }

  const runnerCounts = {};
  agents.forEach(a => { const r = a.runner || 'claude'; runnerCounts[r] = (runnerCounts[r] || 0) + 1; });
  const runnerSub = Object.keys(runnerCounts).length
    ? Object.entries(runnerCounts).map(([k, n]) => `${n} ${k}`).join(' \u00b7 ')
    : 'claude \u00b7 hermes \u00b7 antigravity';

  const instKpiCls = upInsts.length > 0 ? 'accent-ok' : (insts.length > 0 ? 'accent-warn' : '');
  const instKpiVal = insts.length > 0 ? `${upInsts.length}/${insts.length} UP` : 'Absent';
  const instKpiSub = primaryInst
    ? `${primaryInst.label}${primaryInst.version ? ' \u00b7 v' + primaryInst.version : ''}${primaryInst.url ? ' \u00b7 ' + primaryInst.url.replace(/^https?:\/\//, '') : ''}`
    : 'ports 23080\u201323099';

  let bodyHtml = '';
  if (!swarmData && swarmLoading) {
    bodyHtml = `<div class="loading-card" role="status"><span class="spin">${ic('refresh')}</span><span>Interrogation de l'essaim Swarm-control&hellip;</span></div>`;
  } else if (swarmTab === 'hive') {
    bodyHtml = renderSwarmHiveTab(insts, ress, agents, jobs);
  } else if (swarmTab === 'agents') {
    bodyHtml = renderSwarmAgentsTab(insts, agents);
  } else if (swarmTab === 'console') {
    bodyHtml = renderSwarmConsoleTab(insts, ress, agents, jobs);
  } else if (swarmTab === 'runs') {
    bodyHtml = renderSwarmRunsTab(insts, agents, jobs);
  }

  contentEl.innerHTML = `<div class="sw-wrap">
    <div class="sw-head">
      <div class="sw-head-l">
        <div class="sw-head-ico">${swarmIconSvg('ic-hornet')}</div>
        <div>
          <h1 class="sw-head-title">Essaim Swarm <span class="tag swarm">LiteMCP</span><span class="tag sw-a2a">A2A</span>${primaryInst && primaryInst.version ? `<span class="d-meta-pill">v${esc(primaryInst.version)}</span>` : ''}</h1>
          <div class="sw-head-sub">Orchestration multi-agents, sessions MCP r&eacute;sidentes et communications A2A</div>
        </div>
      </div>
      <div class="sw-head-acts">
        <button type="button" data-act="swarm-live-toggle" title="Rafra&icirc;chissement automatique toutes les 5 s">${swarmLive ? '<span class="tag sw-live">LIVE</span> Pause' : 'Reprendre Live'}</button>
        <button type="button" data-act="swarm-refresh" title="Rafra&icirc;chir maintenant">${ic('refresh')} Actualiser</button>
        <button type="button" data-act="scan" title="Scanner les ports locaux (23080&ndash;23099)">${ic('search')} Scanner</button>
        <button type="button" data-act="swarm-sync-logos" title="Synchroniser les logos SVG officiels depuis GitHub">Sync logos</button>
        ${primaryCta}
      </div>
    </div>

    ${renderSwarmBanners(insts, agents, anns)}

    <div class="sw-kpis">
      <div class="sw-kpi ${instKpiCls}">
        <span class="k-lbl">Orchestrateur</span>
        <span class="k-val">${esc(instKpiVal)}</span>
        <span class="k-sub" title="${esc(instKpiSub)}">${esc(instKpiSub)}</span>
      </div>
      <div class="sw-kpi accent-cyan">
        <span class="k-lbl">Agents r&eacute;sidents</span>
        <span class="k-val">${esc(String(resUp))}/${esc(String(ress.length))}</span>
        <span class="k-sub">${ress.length ? 'MCP actifs (7 outils r\u00e9sidents)' : 'aucun r\u00e9sident d\u00e9marr\u00e9'}</span>
      </div>
      <div class="sw-kpi accent-orange">
        <span class="k-lbl">Agents configur&eacute;s</span>
        <span class="k-val">${esc(String(agents.length))}</span>
        <span class="k-sub">${esc(runnerSub)}</span>
      </div>
      <div class="sw-kpi">
        <span class="k-lbl">Activit&eacute; &amp; Flux</span>
        <span class="k-val">${esc(String(runningJobs + activeRuns))} <span class="k-sub">en cours</span></span>
        <span class="k-sub">${anns.length ? esc(String(anns.length)) + ' annonce(s) non suivie(s)' : esc(String(totalTurns)) + ' tour(s) A2A termin\u00e9(s)'}</span>
      </div>
    </div>

    <div class="sw-tabs" role="tablist" aria-label="Sections Swarm Control">
      <button type="button" class="sw-tab ${swarmTab==='hive'?'sel':''}" role="tab" aria-selected="${swarmTab==='hive'?'true':'false'}" data-act="swarm-tab" data-tab="hive">Vue d'ensemble &amp; A2A <span class="sw-tab-badge">${esc(String(ress.length))}</span></button>
      <button type="button" class="sw-tab ${swarmTab==='agents'?'sel':''}" role="tab" aria-selected="${swarmTab==='agents'?'true':'false'}" data-act="swarm-tab" data-tab="agents">Agents persistants <span class="sw-tab-badge">${esc(String(agents.length))}</span></button>
      <button type="button" class="sw-tab ${swarmTab==='console'?'sel':''}" role="tab" aria-selected="${swarmTab==='console'?'true':'false'}" data-act="swarm-tab" data-tab="console">Console &amp; Config</button>
      <button type="button" class="sw-tab ${swarmTab==='runs'?'sel':''}" role="tab" aria-selected="${swarmTab==='runs'?'true':'false'}" data-act="swarm-tab" data-tab="runs">Runs &amp; Parall&egrave;le${(runningJobs + activeRuns) ? ` <span class="sw-tab-badge">${esc(String(runningJobs + activeRuns))}</span>` : ''}</button>
    </div>

    ${bodyHtml}
  </div>`;

  swarmRestoreInputs(contentEl, savedInputs);
  const feedEl = document.getElementById('swConsoleFeed');
  if (feedEl) feedEl.scrollTop = feedEl.scrollHeight;
}

function renderResLogsDrawer(serverId) {
  const lg = swarmResLogsOpen[serverId];
  if (!lg || !lg.open) return '';
  return `<div class="sw-logs-box">
    <div class="sw-logs-head">
      <span>Sortie du tour (${lg.live ? 'EN DIRECT' : 'dernier tour'} &middot; ${esc(String(lg.bytes || 0))} octets)</span>
      <div class="sw-toolbar">
        <button type="button" data-act="swarm-res-logs" data-id="${esc(serverId)}">Fermer</button>
      </div>
    </div>
    <pre class="sw-stream-pre">${escPre(lg.output || '(sortie vide)')}</pre>
  </div>`;
}

function renderSwarmHiveTab(insts, ress, agents, jobs) {
  const edges = [];
  const timeline = [];

  for (const r of ress) {
    const st = swarmResidentLive[r.id] || r.status || {};
    const agName = r.agent || r.label;
    if (st.current && st.current.from) {
      edges.push({from: st.current.from, to: agName, label: `actif (${st.current.elapsedMs || 0} ms)`, active: true});
      timeline.push({
        ts: Date.now(),
        kind: 'a2a',
        from: st.current.from,
        to: agName,
        via: [],
        meta: `EN COURS \u00b7 run ${st.current.runId || '?'} \u00b7 ${st.current.elapsedMs || 0} ms \u00b7 file ${st.queueLength || 0}`,
        body: `Appel A2A actif vers ${agName}`,
        err: false,
      });
    }
    if (st.lastTurn) {
      timeline.push({
        ts: st.lastTurn.at ? Date.parse(st.lastTurn.at) || Date.now() - 1000 : Date.now() - 1000,
        kind: 'a2a',
        from: st.instance || 'swarm',
        to: agName,
        via: [],
        meta: `dernier tour \u00b7 ${st.lastTurn.ok ? 'OK' : 'ERREUR'} \u00b7 ${st.lastTurn.durationMs || 0} ms`,
        body: st.lastTurn.error ? String(st.lastTurn.error) : `Tour termin\u00e9 (${st.turnsDone || 0} au total)`,
        err: !st.lastTurn.ok,
      });
    }
  }

  for (const a of agents) {
    if (Array.isArray(a.mcpServers)) {
      for (const srv of a.mcpServers) {
        const sname = typeof srv === 'string' ? srv : (srv && srv.name ? srv.name : '');
        if (sname) edges.push({from: a.name, to: sname, label: 'mcpServers', active: !!a.resident});
      }
    }
  }

  for (const inst of insts) {
    for (const r of (inst.runs || [])) {
      const rid = r.runId || r.id || '';
      const aname = r.agentName || r.name || 'agent';
      timeline.push({
        ts: Date.now() - 500,
        kind: 'runs',
        from: inst.label,
        to: aname,
        via: [],
        meta: `run ${r.status || 'running'} \u00b7 ${rid}`,
        body: `Ex\u00e9cution sur ${inst.label}`,
        err: r.status === 'error' || r.status === 'failed',
        instId: inst.id,
        runId: rid,
        agentName: aname,
      });
    }
  }

  for (const j of jobs) {
    const isPar = j.kind === 'run_parallel';
    const r = j.result || {};
    const via = Array.isArray(r.via) ? r.via : [];
    if (via.length > 1) {
      for (let i = 0; i < via.length - 1; i++) {
        edges.push({from: via[i], to: via[i + 1], label: 'via A2A', active: j.status === 'running'});
      }
    }
    timeline.push({
      ts: (j.started_ts || 0) * 1000,
      kind: isPar ? 'orch' : 'a2a',
      from: 'mcp-control',
      to: j.agent || j.server_id,
      via,
      meta: `${j.kind} \u00b7 ${j.status}${j.elapsed_ms != null ? ' \u00b7 ' + j.elapsed_ms + ' ms' : ''}`,
      body: j.error ? j.error : (typeof r.reply === 'string' ? r.reply.slice(0, 280) : (j.prompt_preview || '')),
      err: j.status === 'error',
    });
  }

  for (const ev of swarmA2aEvents) timeline.push(ev);
  timeline.sort((a, b) => (b.ts || 0) - (a.ts || 0));

  const filteredEvents = timeline.filter(ev => {
    if (swarmA2aFilter === 'all') return true;
    if (swarmA2aFilter === 'err') return !!ev.err;
    return ev.kind === swarmA2aFilter;
  });

  const instNodes = insts.map(inst => {
    const up = inst.state === 'UP';
    return `<div class="sw-node is-inst">
      <div class="sw-node-top">
        <div class="sw-node-l"><span class="dot ${esc(inst.state)}"></span>${swarmIconSvg('ic-hornet', inst.label, 'instance')}<span>${esc(inst.label)}</span></div>
        <span class="tag ${up ? 'sw-idle' : 'sw-off'}">${esc(inst.state)}</span>
      </div>
      <div class="sw-node-meta">
        <span class="tag swarm">ORCHESTRATEUR</span>
        ${inst.version ? `<span>v<b>${esc(inst.version)}</b></span>` : ''}
        ${inst.url ? `<span class="srow-pill">${esc(inst.url.replace(/^https?:\/\//, ''))}</span>` : ''}
        <span>agents : <b>${esc(String((inst.agents || []).length))}</b></span>
      </div>
      <div class="sw-toolbar">
        <button type="button" data-act="select" data-id="${esc(inst.id)}">Fiche MCP</button>
        ${up ? `<button type="button" data-act="swarm-quick-create">+ Cr&eacute;er agent</button>` : `<button type="button" class="primary" data-act="row-act" data-op="start" data-id="${esc(inst.id)}">${ic('play')} D&eacute;marrer</button>`}
      </div>
    </div>`;
  }).join('');

  const resNodes = ress.map(r => {
    const st = swarmResidentLive[r.id] || r.status || {};
    const busy = st.state === 'busy';
    const agName = r.agent || r.label;
    const agCls = st.class || r.swarm_class || '';
    const needsPair = r.locked && !r.paired;
    const stateLbl = (st.state || (r.state === 'UP' ? 'idle' : 'off')).toUpperCase();
    return `<div class="sw-node is-res ${busy ? 'is-busy' : ''}">
      <div class="sw-node-top">
        <div class="sw-node-l"><span class="dot ${esc(r.state)}"></span>${swarmIconSvg('ic-hornet', agName, 'resident', agCls)}<span>${esc(agName)}</span></div>
        <span class="tag ${busy ? 'sw-busy' : (r.state === 'UP' ? 'sw-idle' : 'sw-off')}">${esc(stateLbl)}</span>
      </div>
      <div class="sw-node-meta">
        ${agCls ? `<span class="tag swarm-res">${esc(agCls)}</span>` : '<span class="tag swarm-res">r&eacute;sident</span>'}
        ${st.runner ? `<span class="tag sw-runner">${esc(st.runner)}</span>` : ''}
        <span>file : <b>${esc(String(st.queueLength || 0))}</b></span>
        <span>tours : <b>${esc(String(st.turnsDone || 0))}</b></span>
        ${st.session ? `<span class="d-meta-pill" title="Session ${esc(String(st.session))}">sess ${esc(String(st.session).slice(0, 8))}</span>` : ''}
      </div>
      ${busy && st.current ? `<div class="sw-edge is-active"><b>${esc(st.current.from || '?')}</b><span class="sw-arrow">&rarr;</span><b>${esc(agName)}</b><span>${esc(String(st.current.elapsedMs || 0))} ms</span></div>` : ''}
      ${r.error ? `<div class="sw-card-err">${esc(r.error)}</div>` : ''}
      ${renderResLogsDrawer(r.id)}
      <div class="sw-toolbar">
        ${needsPair ? `<button type="button" class="primary" data-act="server-pair" data-id="${esc(r.id)}">${ic('lock')} Appairer</button>` : ''}
        <button type="button" class="primary" data-act="swarm-open-console" data-target="res:${esc(r.id)}">Dialoguer</button>
        ${r.state === 'UP' ? `<button type="button" data-act="swarm-res-logs" data-id="${esc(r.id)}">Logs</button>` : ''}
        <button type="button" data-act="select" data-id="${esc(r.id)}">Fiche</button>
        ${r.state === 'UP' ? `<button type="button" class="danger" data-act="swarm-res-stop" data-id="${esc(r.id)}" data-clear="0">Stop</button>` : ''}
      </div>
    </div>`;
  }).join('');

  const inactiveAgents = agents.filter(a => !a.resident && !swarmFindResidentFor(a.name));
  const emptyResHint = !ress.length
    ? (inactiveAgents.length
        ? `<div class="sw-empty"><div><b>${esc(String(inactiveAgents.length))}</b> agent(s) configur&eacute;(s), aucun d&eacute;marr&eacute; en mode r&eacute;sident.</div><div class="sw-empty-acts"><button type="button" class="primary" data-act="swarm-tab" data-tab="agents">D&eacute;marrer un agent r&eacute;sident</button></div></div>`
        : `<div class="sw-empty"><div>Aucun agent r&eacute;sident actif pour le moment.</div>${insts.some(i => i.state === 'UP') ? `<div class="sw-empty-acts"><button type="button" class="primary" data-act="swarm-quick-create">+ Cr&eacute;er un agent r&eacute;sident</button></div>` : ''}</div>`)
    : '';

  const colEdges = edges.length
    ? edges.slice(0, 24).map(e => `<div class="sw-edge ${e.active ? 'is-active' : ''}">
        <b>${esc(e.from)}</b><span class="sw-arrow">&rarr;</span><b>${esc(e.to)}</b>
        <span class="d-meta-pill">${esc(e.label)}</span>
      </div>`).join('')
    : `<div class="sw-empty"><div>Aucun lien A2A actif ou serveur MCP c&acirc;bl&eacute; dans <code>.mcp.json</code>.</div>${agents.length ? `<div class="sw-empty-acts"><button type="button" data-act="swarm-tab" data-tab="console">C&acirc;bler des serveurs MCP</button></div>` : ''}</div>`;

  const resOpts = ress.filter(r => r.state === 'UP').map(r =>
    `<option value="${esc(r.id)}">${esc(r.agent || r.label)}</option>`
  ).join('');

  const timelineHtml = filteredEvents.length
    ? filteredEvents.slice(0, 40).map(ev => {
        const cls = ev.err ? 'ev-err' : (ev.kind === 'orch' ? 'ev-orch' : (ev.kind === 'runs' ? 'ev-run' : 'ev-a2a'));
        const viaHtml = (ev.via && ev.via.length)
          ? `<span class="d-meta-pill">via ${esc(ev.via.join(' \u2192 '))}</span>`
          : '';
        return `<div class="sw-ev ${cls}">
          <div class="sw-ev-top">
            <div class="sw-ev-route">
              <span class="tag ${ev.kind === 'orch' ? 'swarm' : (ev.kind === 'runs' ? 'swarm-res' : 'sw-a2a')}">${esc(ev.kind.toUpperCase())}</span>
              <span>${esc(ev.from)}</span><span class="sw-arrow">&rarr;</span><span>${esc(ev.to)}</span>
              ${viaHtml}
            </div>
            <div class="sw-toolbar">
              <span class="d-meta-pill">${esc(ev.meta || '')}</span>
              ${ev.runId && ev.instId ? `<button type="button" data-act="swarm-run-ctrl" data-inst="${esc(ev.instId)}" data-op="stream" data-runid="${esc(ev.runId)}" data-agent="${esc(ev.agentName || '')}">Stream</button><button type="button" class="danger" data-act="swarm-run-ctrl" data-inst="${esc(ev.instId)}" data-op="kill" data-runid="${esc(ev.runId)}" data-agent="${esc(ev.agentName || '')}">Kill</button>` : ''}
            </div>
          </div>
          ${ev.body ? `<pre class="sw-ev-body">${escPre(ev.body)}</pre>` : ''}
        </div>`;
      }).join('')
    : '<div class="sw-empty">Aucune interaction A2A ou ex&eacute;cution enregistr&eacute;e pour ce filtre.</div>';

  return `<div class="sw-topo">
      <div class="sw-topo-col">
        <div class="sw-topo-head"><span>Topologie de l'essaim</span><span>${esc(String(insts.length))} orch. &middot; ${esc(String(ress.length))} r&eacute;sid.</span></div>
        ${instNodes}
        ${resNodes}
        ${emptyResHint}
      </div>
      <div class="sw-topo-col">
        <div class="sw-topo-head"><span>Liens A2A &amp; MCP</span><span>${esc(String(edges.length))}</span></div>
        ${colEdges}
      </div>
    </div>

    <div class="sw-form">
      <div class="sw-sec-head">
        <h2>Interaction A2A rapide (<code>ask</code>)</h2>
        ${!resOpts ? '<span class="d-meta-pill">D&eacute;marrez un agent r&eacute;sident UP pour activer l\'envoi direct</span>' : ''}
      </div>
      <div class="sw-a2a-bar">
        <input id="swA2aFrom" placeholder="Exp&eacute;diteur" value="mcp-control" aria-label="Exp&eacute;diteur A2A" ${resOpts ? '' : 'disabled'}>
        <select id="swA2aTo" aria-label="Agent destinataire A2A" ${resOpts ? '' : 'disabled'}>
          ${resOpts || '<option value="">Aucun r&eacute;sident UP</option>'}
        </select>
        <input id="swA2aVia" placeholder="via (opt. ex: a,b)" aria-label="Cha&icirc;ne via A2A" ${resOpts ? '' : 'disabled'}>
        <input id="swA2aPrompt" placeholder="Message A2A &agrave; envoyer &agrave; l'agent (Entr&eacute;e pour envoyer)&hellip;" aria-label="Message A2A" ${resOpts ? '' : 'disabled'}>
        <button type="button" class="primary" data-act="swarm-a2a-send" ${resOpts ? '' : 'disabled'}>Envoyer A2A</button>
      </div>
    </div>

    <div class="sw-sec-head">
      <h2>Chronologie des interactions (${esc(String(filteredEvents.length))})</h2>
      <div class="sw-toolbar">
        <button type="button" class="exp-pill ${swarmA2aFilter==='all'?'sel':''}" data-act="swarm-a2a-filter" data-mode="all">Tous</button>
        <button type="button" class="exp-pill ${swarmA2aFilter==='a2a'?'sel':''}" data-act="swarm-a2a-filter" data-mode="a2a">A2A</button>
        <button type="button" class="exp-pill ${swarmA2aFilter==='orch'?'sel':''}" data-act="swarm-a2a-filter" data-mode="orch">Parall&egrave;le</button>
        <button type="button" class="exp-pill ${swarmA2aFilter==='runs'?'sel':''}" data-act="swarm-a2a-filter" data-mode="runs">Runs</button>
        <button type="button" class="exp-pill ${swarmA2aFilter==='err'?'sel':''}" data-act="swarm-a2a-filter" data-mode="err">Erreurs</button>
      </div>
    </div>
    <div class="sw-timeline">${timelineHtml}</div>`;
}

function renderClassSelect(agentName, serverId, curCls) {
  const opts = [['auto', 'Logo auto']].concat(SWARM_CLASSES).map(([id, lbl]) => {
    const sel = (id === 'auto' && !curCls) || (id === curCls);
    return `<option value="${esc(id)}"${sel ? ' selected' : ''}>${esc(lbl.split(' ')[0])}</option>`;
  }).join('');
  return `<select class="sw-cls-sel" aria-label="Classe de ${esc(agentName || serverId)}" data-act="swarm-set-cls" data-agent="${esc(agentName || '')}" data-id="${esc(serverId || '')}" title="Changer la classe / le logo officiel de cet agent">${opts}</select>`;
}

function renderSwarmAgentsTab(insts, agents) {
  const upInsts = insts.filter(i => i.state === 'UP');
  const instOpts = insts.map(i =>
    `<option value="${esc(i.id)}"${swarmSelectedInst === i.id ? ' selected' : ''}>${esc(i.label)} (${esc(i.state)})</option>`
  ).join('');

  const filtered = agents.filter(a => {
    if (swarmSelectedInst && insts.length > 1 && a._instanceId !== swarmSelectedInst) return false;
    if (swarmFilterRunner !== 'all' && (a.runner || '') !== swarmFilterRunner) return false;
    if (swarmFilterState === 'resident' && !a.resident) return false;
    if (swarmFilterState === 'running' && !a.running) return false;
    if (swarmFilterState === 'session' && !a.hasSession) return false;
    if (swarmSearch) {
      const hay = ((a.name || '') + ' ' + (a.runner || '') + ' ' + (a.model || '') + ' ' + (a.class || '')).toLowerCase();
      if (!hay.includes(swarmSearch.toLowerCase())) return false;
    }
    return true;
  });

  const clsOpts = SWARM_CLASSES.map(([id, lbl]) =>
    `<option value="${esc(id)}"${swarmNewClass === id ? ' selected' : ''}>${esc(lbl)}</option>`
  ).join('');

  let createHtml = '';
  if (swarmCreateOpen || (upInsts.length > 0 && agents.length === 0)) {
    createHtml = `<div class="sw-form">
      <div class="sw-sec-head">
        <h2><span id="swNewClsPrev" class="sw-cls-prev">${swarmIconSvg('ic-hornet', swarmNewClass, 'resident', swarmNewClass)}</span> Cr&eacute;er un agent Swarm-control</h2>
        ${agents.length > 0 ? `<button type="button" data-act="swarm-toggle-create">Fermer</button>` : ''}
      </div>
      <div class="sw-form-row">
        <div class="sw-field"><label for="swNewName">Nom de l'agent</label><input id="swNewName" placeholder="ex. alice, reviewer, ops"></div>
        <div class="sw-field"><label for="swNewClass">Classe / Logo</label><select id="swNewClass" data-act="swarm-new-cls">${clsOpts}</select></div>
        <div class="sw-field"><label for="swNewRunner">Runner</label><select id="swNewRunner"><option value="claude">claude</option><option value="hermes">hermes</option><option value="antigravity">antigravity</option></select></div>
        <div class="sw-field"><label for="swNewModel">Mod&egrave;le (optionnel)</label><input id="swNewModel" placeholder="ex. sonnet, opus, gemini-2.5-pro"></div>
        <div class="sw-field"><label for="swNewClone">Cloner / Copier env (opt.)</label><input id="swNewClone" placeholder="profil Hermes ou settings.json"></div>
      </div>
      <div class="sw-field"><label for="swNewPrompt">Prompt syst&egrave;me (<code>prompt.md</code> / <code>SOUL.md</code>)</label><textarea id="swNewPrompt" placeholder="Décrivez la mission, les outils et les consignes de cet agent&hellip;"></textarea></div>
      <div class="sw-toolbar">
        <label class="sw-check"><input type="checkbox" id="swNewResident" checked> D&eacute;marrer imm&eacute;diatement en agent r&eacute;sident MCP</label>
        <label class="sw-check"><input type="checkbox" id="swNewExe"> Cr&eacute;er aussi un binaire autonome (<code>bin/&lt;nom&gt;</code>)</label>
        <button type="button" class="primary" data-act="swarm-create-submit" ${upInsts.length ? '' : 'disabled'}>Cr&eacute;er l'agent</button>
      </div>
    </div>`;
  }

  let cardsHtml = '';
  if (!upInsts.length) {
    cardsHtml = `<div class="sw-empty">Aucune instance Swarm-control UP disponible pour lister ou cr&eacute;er des agents.</div>`;
  } else if (!agents.length) {
    cardsHtml = '';
  } else if (!filtered.length) {
    cardsHtml = `<div class="sw-empty">Aucun agent ne correspond aux filtres courants.</div>`;
  } else {
    cardsHtml = '<div class="sw-grid">';
    for (const a of filtered) {
      const matchedRes = swarmFindResidentFor(a.name);
      const isRes = Boolean(a.resident || (matchedRes && matchedRes.state === 'UP'));
      const resUrl = (a.resident && a.resident.url) || (matchedRes && matchedRes.url) || '';
      const unfollowedUrl = (a.resident && a.resident.url && !matchedRes) ? a.resident.url : '';
      const consoleTgt = matchedRes
        ? 'res:' + matchedRes.id
        : 'inst:' + a._instanceId + ':' + a.name;
      const aCls = a.class || (matchedRes && (matchedRes.swarm_class || (matchedRes.status && matchedRes.status.class))) || '';
      const mcpCount = Array.isArray(a.mcpServers) ? a.mcpServers.length : 0;
      const promptKb = a.promptBytes != null ? (a.promptBytes >= 1024 ? (a.promptBytes / 1024).toFixed(1) + ' Ko' : a.promptBytes + ' o') : '';
      cardsHtml += `<div class="sw-card ${isRes ? 'is-resident' : ''}">
        <div class="sw-card-head">
          <div class="sw-card-title">${swarmIconSvg('ic-hornet', a.name, 'resident', aCls)}<span class="nm">${esc(a.name)}</span></div>
          <div class="sw-toolbar">
            ${renderClassSelect(a.name, matchedRes ? matchedRes.id : '', aCls)}
            ${isRes ? '<span class="tag swarm-res">R&Eacute;SIDENT ON</span>' : '<span class="d-meta-pill">inactif</span>'}
          </div>
        </div>
        <div class="sw-card-meta">
          <span class="tag sw-runner">${esc(a.runner || '?')}</span>
          ${a.model ? `<span class="d-meta-pill">${esc(a.model)}</span>` : ''}
          ${promptKb ? `<span class="d-meta-pill" title="Taille du prompt">${esc(promptKb)}</span>` : ''}
          ${mcpCount ? `<span class="d-meta-pill" title="${esc(a.mcpServers.join(', '))}">${esc(String(mcpCount))} MCP</span>` : ''}
          ${a.hasSession ? `<span class="tag sw-idle" title="${esc(a.sessionId || 'session active')}">session</span>` : ''}
          ${a.running ? '<span class="tag sw-busy">running</span>' : ''}
          ${resUrl ? `<span class="srow-pill">${esc(resUrl.replace(/^https?:\/\//, ''))}</span>` : ''}
        </div>
        ${a.warning ? `<div class="sw-card-err">${esc(a.warning)}</div>` : ''}
        <div class="sw-card-acts">
          ${isRes
            ? `<button type="button" class="danger" data-act="swarm-agent-stop" data-inst="${esc(a._instanceId)}" data-name="${esc(a.name)}" data-runner="${esc(a.runner || '')}">Stopper</button>`
            : `<button type="button" class="primary" data-act="swarm-agent-start" data-inst="${esc(a._instanceId)}" data-name="${esc(a.name)}" data-runner="${esc(a.runner || '')}">D&eacute;marrer r&eacute;sident</button>`}
          ${unfollowedUrl ? `<button type="button" class="primary" data-act="swarm-follow-url" data-url="${esc(unfollowedUrl)}" data-name="${esc(a.name)}">Suivre</button>` : ''}
          <button type="button" data-act="swarm-open-console" data-target="${esc(consoleTgt)}" data-agent="${esc(a.name)}" data-inst="${esc(a._instanceId)}">Console</button>
          <button type="button" data-act="swarm-load-cfg" data-inst="${esc(a._instanceId)}" data-name="${esc(a.name)}" data-runner="${esc(a.runner || '')}">Config</button>
          <button type="button" class="danger" data-act="swarm-agent-delete" data-inst="${esc(a._instanceId)}" data-name="${esc(a.name)}" data-runner="${esc(a.runner || '')}">Supprimer</button>
        </div>
      </div>`;
    }
    cardsHtml += '</div>';
  }

  return `<div class="sw-sec-head">
      <div class="sw-toolbar">
        ${insts.length > 1 ? `<select id="swInstSel" aria-label="Instance Swarm" data-act="swarm-inst-change">${instOpts}</select>` : ''}
        <select id="swRunFilter" aria-label="Filtrer par runner" data-act="swarm-filter-change">
          <option value="all"${swarmFilterRunner==='all'?' selected':''}>Tous runners</option>
          <option value="claude"${swarmFilterRunner==='claude'?' selected':''}>claude</option>
          <option value="hermes"${swarmFilterRunner==='hermes'?' selected':''}>hermes</option>
          <option value="antigravity"${swarmFilterRunner==='antigravity'?' selected':''}>antigravity</option>
        </select>
        <select id="swStateFilter" aria-label="Filtrer par &eacute;tat" data-act="swarm-filter-change">
          <option value="all"${swarmFilterState==='all'?' selected':''}>Tous &eacute;tats</option>
          <option value="resident"${swarmFilterState==='resident'?' selected':''}>R&eacute;sidents actifs</option>
          <option value="running"${swarmFilterState==='running'?' selected':''}>En cours</option>
          <option value="session"${swarmFilterState==='session'?' selected':''}>Avec session</option>
        </select>
        <input id="swAgentSearch" placeholder="Rechercher un agent&hellip;" value="${esc(swarmSearch)}" data-act="swarm-filter-change">
      </div>
      ${agents.length > 0 ? `<button type="button" class="primary" data-act="swarm-toggle-create">${swarmCreateOpen ? 'Fermer formulaire' : '+ Nouvel agent'}</button>` : ''}
    </div>
    ${createHtml}
    ${cardsHtml}`;
}

function swarmWiredMcpSet(filesMap) {
  const wired = new Set();
  if (!filesMap || filesMap['.mcp.json'] == null) return wired;
  try {
    const raw = filesMap['.mcp.json'];
    const parsed = typeof raw === 'string' ? JSON.parse(raw) : raw;
    const map = (parsed && (parsed.mcpServers || parsed.servers)) || {};
    Object.keys(map).forEach(k => wired.add(k.toLowerCase()));
  } catch (_) {}
  return wired;
}

function swarmAvailableCfgFiles(filesMap, runner) {
  const keys = Object.keys(filesMap || {});
  if (!keys.length) {
    if (runner === 'antigravity') return ['prompt.md', 'settings.json'];
    if (runner === 'hermes') return ['prompt.md', 'settings.json', '.mcp.json', '.env'];
    return ['prompt.md', 'settings.json', '.mcp.json', 'skill.md'];
  }
  const order = ['prompt.md', 'settings.json', '.mcp.json', 'skill.md', '.env'];
  const sorted = order.filter(k => keys.includes(k));
  keys.forEach(k => { if (!sorted.includes(k)) sorted.push(k); });
  if (runner === 'claude' && !sorted.includes('skill.md')) sorted.push('skill.md');
  return sorted;
}

function renderSwarmConsoleTab(insts, ress, agents, jobs) {
  const targetOpts = [];
  for (const r of ress) {
    targetOpts.push(`<option value="res:${esc(r.id)}"${swarmConsoleTarget === 'res:' + r.id ? ' selected' : ''}>[R\u00e9sident] ${esc(r.agent || r.label)} (${esc(r.state)})</option>`);
  }
  for (const a of agents) {
    const val = 'inst:' + a._instanceId + ':' + a.name;
    targetOpts.push(`<option value="${esc(val)}"${swarmConsoleTarget === val ? ' selected' : ''}>[Run] ${esc(a.name)} (${esc(a.runner || '?')})</option>`);
  }

  let selectedRes = null;
  if (swarmConsoleTarget.startsWith('res:')) {
    const rid = swarmConsoleTarget.slice(4);
    selectedRes = ress.find(r => r.id === rid) || null;
  }

  let statusBar = '';
  let histMoreBtn = '';
  let logsDrawer = '';
  if (selectedRes) {
    const st = swarmResidentLive[selectedRes.id] || selectedRes.status || {};
    const stName = st.state || (selectedRes.state === 'UP' ? 'idle' : 'offline');
    const hInfo = swarmHistLoaded[selectedRes.id];
    if (hInfo && hInfo.hasMore) {
      histMoreBtn = `<div class="sw-empty-acts"><button type="button" data-act="swarm-res-hist-more" data-id="${esc(selectedRes.id)}">Charger les tours plus anciens</button></div>`;
    }
    statusBar = `<div class="sw-status-bar">
      <span class="tag ${stName === 'busy' ? 'sw-busy' : (selectedRes.state === 'UP' ? 'sw-idle' : 'sw-off')}">${esc(stName.toUpperCase())}</span>
      <span>file : <b>${esc(String(st.queueLength || 0))}</b></span>
      <span>tours : <b>${esc(String(st.turnsDone || 0))}</b></span>
      ${st.session ? `<span class="d-meta-pill" title="Session ${esc(String(st.session))}">session ${esc(String(st.session).slice(0, 12))}</span>` : '<span class="d-meta-pill">sans session</span>'}
      ${st.current && st.current.from ? `<span>en cours (${esc(st.current.from)}) ${esc(String(st.current.elapsedMs || 0))} ms</span>` : ''}
      <button type="button" data-act="swarm-res-status" data-id="${esc(selectedRes.id)}">Statut</button>
      <button type="button" data-act="swarm-res-logs" data-id="${esc(selectedRes.id)}">Logs</button>
      <button type="button" data-act="swarm-res-sess-reset" data-id="${esc(selectedRes.id)}" title="Oublier la session courante">Nouvelle session</button>
      <button type="button" data-act="swarm-res-sess-set" data-id="${esc(selectedRes.id)}" title="Forcer un ID de session">Session&hellip;</button>
      <button type="button" class="danger" data-act="swarm-res-stop" data-id="${esc(selectedRes.id)}" data-clear="0">Stop</button>
      <button type="button" class="danger" data-act="swarm-res-stop" data-id="${esc(selectedRes.id)}" data-clear="1">Vider file</button>
      ${selectedRes.bridge_port ? `<button type="button" data-act="swarm-bridge-op" data-port="${esc(String(selectedRes.bridge_port))}" data-op="status">Bridge :${esc(String(selectedRes.bridge_port))}</button><button type="button" data-act="swarm-bridge-op" data-port="${esc(String(selectedRes.bridge_port))}" data-op="reset">Reset bridge</button>` : ''}
    </div>`;
    logsDrawer = renderResLogsDrawer(selectedRes.id);
  }

  const runningAsk = jobs.filter(j => (j.kind === 'ask' || j.kind === 'run_agent') && j.status === 'running');
  const feed = swarmFeedFor(swarmConsoleTarget);
  const feedHtml = feed.length
    ? histMoreBtn + feed.map(m => `<div class="sw-msg ${m.role === 'user' ? 'is-user' : ''} ${m.err ? 'is-err' : ''}">
        <div class="sw-msg-head"><b>${esc(m.title)}</b><span>${esc(m.meta || '')}</span></div>
        <pre>${escPre(m.text)}</pre>
      </div>`).join('')
    : '<div class="sw-empty">Envoyez un prompt &agrave; un agent r&eacute;sident (<code>ask</code>) ou via l\'orchestrateur (<code>run_agent</code>).</div>';

  const cfgAgObj = swarmCfgAgent ? agents.find(a => (a.name || '').toLowerCase() === swarmCfgAgent.toLowerCase()) : null;
  const activeRunner = (swarmCfgData && swarmCfgData.runner) || (cfgAgObj && cfgAgObj.runner) || 'claude';
  const filesMap = (swarmCfgData && swarmCfgData.files) || {};
  const cfgFiles = swarmAvailableCfgFiles(filesMap, activeRunner);
  if (!cfgFiles.includes(swarmCfgFile)) swarmCfgFile = cfgFiles[0] || 'prompt.md';
  const isReadOnlyFile = Boolean((swarmCfgData && swarmCfgData.read_only) || swarmCfgFile === '.env');
  const curContent = filesMap[swarmCfgFile] != null
    ? (typeof filesMap[swarmCfgFile] === 'string' ? filesMap[swarmCfgFile] : JSON.stringify(filesMap[swarmCfgFile], null, 2))
    : '';
  const wiredSet = swarmWiredMcpSet(filesMap);
  const mcpTargets = ((swarmData && swarmData.mcp_targets) || []).filter(t => !t.swarm_kind || t.swarm_kind === 'resident');
  const mcpChecks = mcpTargets.slice(0, 24).map(t => {
    const isChecked = wiredSet.has(String(t.label || '').toLowerCase()) || wiredSet.has(String(t.id || '').toLowerCase());
    return `<label class="sw-mcp-chip"><input type="checkbox" class="sw-mcp-cb" value="${esc(t.label)}"${isChecked ? ' checked' : ''}> ${esc(t.label)}</label>`;
  }).join('');

  return `<div class="sw-split">
    <div class="sw-console-box">
      <div class="sw-sec-head">
        <h2>Console interactive</h2>
        <select id="swConsoleTarget" aria-label="Cible console" data-act="swarm-console-target">
          ${targetOpts.length ? targetOpts.join('') : '<option value="">Aucun agent disponible</option>'}
        </select>
      </div>
      ${statusBar}
      ${logsDrawer}
      ${runningAsk.length ? `<div class="sw-status-bar"><span class="spin">${ic('refresh')}</span><span>${esc(String(runningAsk.length))} requ&ecirc;te(s) en cours&hellip;</span></div>` : ''}
      <div class="sw-feed" id="swConsoleFeed">${feedHtml}</div>
      <div class="sw-field">
        <label for="swAskPrompt">Prompt <span class="d-meta-pill">Ctrl+Entr&eacute;e</span></label>
        <textarea id="swAskPrompt" placeholder="Votre message pour l'agent&hellip;" ${targetOpts.length ? '' : 'disabled'}></textarea>
      </div>
      <div class="sw-toolbar">
        <input id="swAskFrom" placeholder="from (ex. mcp-control)" value="mcp-control" ${targetOpts.length ? '' : 'disabled'}>
        <input id="swAskModel" placeholder="mod&egrave;le (opt.)" ${targetOpts.length ? '' : 'disabled'}>
        ${swarmConsoleTarget.startsWith('inst:') ? `<input id="swAskPath" placeholder="workdir path (opt.)" ${targetOpts.length ? '' : 'disabled'}>` : ''}
        <button type="button" class="primary" data-act="swarm-send-ask" ${targetOpts.length ? '' : 'disabled'}>Envoyer</button>
        <button type="button" data-act="swarm-clear-feed">Effacer vue</button>
      </div>
    </div>

    <div class="sw-console-box">
      <div class="sw-sec-head">
        <h2>Configuration${swarmCfgAgent ? ' : ' + esc(swarmCfgAgent) : ''}${swarmCfgData && swarmCfgData.read_only ? ' <span class="d-meta-pill">lecture seule</span>' : ''}</h2>
        <div class="sw-toolbar">
          <select id="swCfgAgentSel" aria-label="Agent &agrave; configurer" data-act="swarm-cfg-pick">
            <option value="">Choisir un agent&hellip;</option>
            ${agents.map(a => `<option value="${esc(a._instanceId)}|${esc(a.name)}|${esc(a.runner || '')}"${swarmCfgAgent === a.name ? ' selected' : ''}>${esc(a.name)} (${esc(a.runner || '?')})</option>`).join('')}
          </select>
        </div>
      </div>
      <div class="sw-tabs">
        ${cfgFiles.map(f => `<button type="button" class="sw-tab ${swarmCfgFile===f?'sel':''}" data-act="swarm-cfg-file" data-file="${esc(f)}">${esc(f)}</button>`).join('')}
      </div>
      <div class="sw-field">
        <label for="swCfgEditor">${esc(swarmCfgFile)}${activeRunner === 'hermes' && swarmCfgFile === 'prompt.md' ? ' (SOUL.md)' : (activeRunner === 'hermes' && swarmCfgFile === 'settings.json' ? ' (config.yaml)' : '')}</label>
        <textarea id="swCfgEditor" placeholder="S&eacute;lectionnez un agent ci-dessus pour lire et modifier sa configuration&hellip;" ${(!swarmCfgAgent || isReadOnlyFile) ? 'disabled' : ''}>${esc(curContent)}</textarea>
      </div>
      <div class="sw-toolbar">
        <button type="button" class="primary" data-act="swarm-cfg-save" ${(!swarmCfgAgent || isReadOnlyFile) ? 'disabled' : ''}>Enregistrer ${esc(swarmCfgFile)}</button>
      </div>
      ${(mcpTargets.length && activeRunner !== 'antigravity') ? `<div class="sw-field">
        <label>Serveurs MCP Control c&acirc;bl&eacute;s dans <code>mcpServers</code></label>
        <div class="sw-mcp-list">${mcpChecks}</div>
        <div><button type="button" data-act="swarm-cfg-wire-mcp" ${(!swarmCfgAgent || (swarmCfgData && swarmCfgData.read_only)) ? 'disabled' : ''}>Appliquer les serveurs coch&eacute;s</button></div>
      </div>` : ''}
    </div>
  </div>`;
}

function renderSwarmRunsTab(insts, agents, jobs) {
  const upInsts = insts.filter(i => i.state === 'UP');
  const instOpts = insts.map(i =>
    `<option value="${esc(i.id)}"${swarmSelectedInst === i.id ? ' selected' : ''}>${esc(i.label)} (${esc(i.state)})</option>`
  ).join('');

  const agentListHtml = `<datalist id="swKnownAgentsList">${agents.map(a => `<option value="${esc(a.name)}"></option>`).join('')}</datalist>`;

  const parRows = swarmParallelDraft.map((row, idx) => `<div class="sw-par-row">
    <select class="sw-par-runner" data-idx="${idx}" aria-label="Runner t&acirc;che ${idx + 1}">
      <option value="claude"${row.runner==='claude'?' selected':''}>claude</option>
      <option value="hermes"${row.runner==='hermes'?' selected':''}>hermes</option>
      <option value="antigravity"${row.runner==='antigravity'?' selected':''}>antigravity</option>
    </select>
    <input class="sw-par-agent" data-idx="${idx}" list="swKnownAgentsList" placeholder="agent (opt.)" value="${esc(row.agentName || '')}">
    <input class="sw-par-prompt" data-idx="${idx}" placeholder="Prompt pour la t&acirc;che #${idx + 1}&hellip;" value="${esc(row.prompt || '')}">
    <button type="button" class="danger" data-act="swarm-par-del" data-idx="${idx}" aria-label="Retirer la t&acirc;che ${idx + 1}" ${swarmParallelDraft.length <= 1 ? 'disabled' : ''}>&times;</button>
  </div>`).join('');

  const allRuns = [];
  for (const inst of insts) {
    for (const r of (inst.runs || [])) {
      allRuns.push(Object.assign({}, r, {_instanceId: inst.id, _instanceLabel: inst.label}));
    }
  }

  const runsHtml = allRuns.length
    ? allRuns.map(r => {
        const rid = r.runId || r.id || '';
        const aname = r.agentName || r.name || '';
        const st = r.status || r.state || '?';
        return `<div class="sw-ann-row">
          <div><span class="tag ${st==='running'?'sw-busy':'sw-idle'}">${esc(st)}</span> <b>${esc(aname || rid)}</b> ${rid ? `<span class="srow-pill">${esc(rid)}</span>` : ''} <span class="d-meta-pill">${esc(r._instanceLabel)}</span></div>
          <div class="sw-toolbar">
            <button type="button" data-act="swarm-run-ctrl" data-inst="${esc(r._instanceId)}" data-op="stream" data-runid="${esc(rid)}" data-agent="${esc(aname)}">Stream</button>
            <button type="button" class="danger" data-act="swarm-run-ctrl" data-inst="${esc(r._instanceId)}" data-op="kill" data-runid="${esc(rid)}" data-agent="${esc(aname)}">Kill</button>
          </div>
        </div>`;
      }).join('')
    : '<div class="sw-empty">Aucun run actif report&eacute; par Swarm-control.</div>';

  const jobsHtml = jobs.length
    ? jobs.slice(0, 15).map(j => {
        const resTxt = j.error
          ? j.error
          : (j.result ? (typeof j.result === 'string' ? j.result : JSON.stringify(j.result, null, 2)) : '');
        return `<div class="sw-ann-row">
          <details class="sw-job-det">
            <summary>
              <div><span class="tag ${j.status==='running'?'sw-busy':(j.status==='done'?'sw-idle':'sw-off')}">${esc(j.status)}</span> <b>${esc(j.kind)}</b> ${j.agent ? `<span class="d-meta-pill">${esc(j.agent)}</span>` : ''} <span class="srow-pill">${esc(j.id)}</span> ${esc(j.prompt_preview || '')}</div>
              <span class="d-meta-pill">${j.elapsed_ms != null ? esc(String(j.elapsed_ms)) + ' ms' : 'en cours'}</span>
            </summary>
            ${resTxt ? `<pre>${escPre(resTxt)}</pre>` : ''}
          </details>
        </div>`;
      }).join('')
    : '<div class="sw-empty">Aucune t&acirc;che lanc&eacute;e pendant cette session.</div>';

  return `${agentListHtml}<div class="sw-form">
      <div class="sw-sec-head">
        <h2>Ex&eacute;cution parall&egrave;le (<code>run_agents_parallel</code>, jusqu'&agrave; 10 t&acirc;ches)</h2>
        ${insts.length > 1 ? `<select id="swParInst" aria-label="Instance cible">${instOpts}</select>` : ''}
      </div>
      ${parRows}
      <div class="sw-toolbar">
        <button type="button" data-act="swarm-par-add">+ Ajouter une t&acirc;che</button>
        <label class="sw-check"><input type="checkbox" id="swParWait" checked> Attendre tous les r&eacute;sultats (<code>waitAll</code>)</label>
        <button type="button" class="primary" data-act="swarm-par-run" ${upInsts.length ? '' : 'disabled'}>Lancer en parall&egrave;le</button>
      </div>
    </div>

    <div class="sw-sec-head"><h2>Runs actifs (<code>agent_control</code>)</h2></div>
    ${runsHtml}
    ${swarmStreamOut ? `<div class="sw-form">
      <div class="sw-sec-head"><h2>Stream : ${esc(swarmStreamOut.agentName || swarmStreamOut.runId)}</h2><button type="button" data-act="swarm-stream-close">Fermer</button></div>
      <pre class="sw-stream-pre">${escPre(swarmStreamOut.text)}</pre>
    </div>` : ''}

    <div class="sw-sec-head"><h2>Historique des t&acirc;ches de la session</h2></div>
    ${jobsHtml}`;
}

/* ===== Vues intégrées dans la fiche Détail d'un agent résident Swarm ===== */

function swarmFindInstanceForAgent(agentName) {
  const insts = (swarmData && swarmData.instances) || [];
  if (agentName) {
    const low = String(agentName).toLowerCase();
    const match = insts.find(i => i.state === 'UP' && (i.agents || []).some(a => (a.name || '').toLowerCase() === low));
    if (match) return match.id;
  }
  const up = insts.find(i => i.state === 'UP');
  return up ? up.id : '';
}

async function swarmOnDetailTab(tab, s) {
  if (!s || s.swarm_kind !== 'resident') return;
  if (tab !== 'swarm-chat' && tab !== 'swarm-cfg') return;
  if (!swarmData) await swarmRefresh(false);
  if (tab === 'swarm-chat') {
    swarmConsoleTarget = 'res:' + s.id;
    if (s.state === 'UP') {
      const [, j] = await Promise.all([
        swarmEnsureResidentHistory(s.id),
        postAction({action: 'swarm_resident_status', server_id: s.id}, {quiet: true}),
      ]);
      if (j && j.ok && j.data) swarmResidentLive[s.id] = j.data;
    }
    swarmEnsurePoll();
    swarmRerenderActive();
  } else if (tab === 'swarm-cfg') {
    const instId = s.swarm_agent ? swarmFindInstanceForAgent(s.swarm_agent) : '';
    if (instId && s.swarm_agent) {
      await swarmLoadConfig(instId, s.swarm_agent, '', true);
    } else if (s.state === 'UP') {
      // Repli autonome via l'outil MCP `config` du résident lui-même
      const j = await postAction({action: 'swarm_resident_config', server_id: s.id}, {quiet: true});
      if (j && j.ok && j.data) {
        swarmCfgAgent = j.data.name || s.swarm_agent || s.label;
        swarmCfgData = j.data;
      }
    }
    swarmRerenderActive();
  }
}

function renderSwarmDetailChat(s) {
  const ress = (swarmData && swarmData.residents) || [];
  const jobs = (swarmData && swarmData.jobs) || [];
  const resEntry = ress.find(r => r.id === s.id);
  const st = swarmResidentLive[s.id] || (resEntry && resEntry.status) || {};
  const target = 'res:' + s.id;
  swarmConsoleTarget = target;

  const hInfo = swarmHistLoaded[s.id];
  const histMoreBtn = (hInfo && hInfo.hasMore)
    ? `<div class="sw-empty-acts"><button type="button" data-act="swarm-res-hist-more" data-id="${esc(s.id)}">Charger les tours plus anciens</button></div>`
    : '';
  const feed = swarmFeedFor(target);
  const runningJobs = jobs.filter(j => j.status === 'running' && j.server_id === s.id);
  const feedHtml = feed.length
    ? histMoreBtn + feed.map(m => `<div class="sw-msg ${m.role === 'user' ? 'is-user' : ''} ${m.err ? 'is-err' : ''}">
        <div class="sw-msg-head"><b>${esc(m.title)}</b><span>${esc(m.meta || '')}</span></div>
        <pre>${escPre(m.text)}</pre>
      </div>`).join('')
    : `<div class="sw-empty">Aucune conversation enregistr&eacute;e pour <b>${esc(s.swarm_agent || s.label)}</b>.<br>Envoyez un message ci-dessous pour dialoguer directement avec l'agent.</div>`;

  const stName = st.state || (s.state === 'UP' ? 'idle' : 'offline');
  const barHtml = `<div class="sw-status-bar">
    <span class="tag ${stName === 'busy' ? 'sw-busy' : (s.state === 'UP' ? 'sw-idle' : 'sw-off')}">${esc(stName.toUpperCase())}</span>
    ${st.runner ? `<span class="tag sw-runner">${esc(st.runner)}</span>` : ''}
    <span>file : <b>${esc(String(st.queueLength || 0))}</b></span>
    <span>tours : <b>${esc(String(st.turnsDone || 0))}</b></span>
    ${st.session ? `<span class="d-meta-pill" title="Session ${esc(String(st.session))}">session ${esc(String(st.session).slice(0, 12))}</span>` : '<span class="d-meta-pill">sans session</span>'}
    ${st.current && st.current.from ? `<span>en cours (${esc(st.current.from)}) ${esc(String(st.current.elapsedMs || 0))} ms</span>` : ''}
    <button type="button" data-act="swarm-res-status" data-id="${esc(s.id)}">Statut</button>
    ${s.state === 'UP' ? `<button type="button" data-act="swarm-res-logs" data-id="${esc(s.id)}">Logs</button><button type="button" data-act="swarm-res-sess-reset" data-id="${esc(s.id)}">Nouvelle session</button><button type="button" data-act="swarm-res-sess-set" data-id="${esc(s.id)}">Session&hellip;</button><button type="button" class="danger" data-act="swarm-res-stop" data-id="${esc(s.id)}" data-clear="0">Stop</button><button type="button" class="danger" data-act="swarm-res-stop" data-id="${esc(s.id)}" data-clear="1">Vider file</button>` : ''}
    ${s.swarm_bridge_port ? `<button type="button" data-act="swarm-bridge-op" data-port="${esc(String(s.swarm_bridge_port))}" data-op="status">Bridge :${esc(String(s.swarm_bridge_port))}</button><button type="button" data-act="swarm-bridge-op" data-port="${esc(String(s.swarm_bridge_port))}" data-op="reset">Reset bridge</button>` : ''}
  </div>`;

  const chatCls = st.class || s.swarm_class || (resEntry && resEntry.swarm_class) || '';
  return `<div class="sw-detail-pane">
    <div class="sw-console-box">
      <div class="sw-sec-head">
        <h2>${swarmIconSvg('ic-hornet', s.swarm_agent || s.label, 'resident', chatCls)} Conversation &mdash; ${esc(s.swarm_agent || s.label)}</h2>
        <div class="sw-toolbar">
          ${renderClassSelect(s.swarm_agent || '', s.id, chatCls)}
          <button type="button" data-act="swarm-open-console" data-target="${esc(target)}" data-inst="${esc(s.id)}">Ouvrir dans Essaim Swarm</button>
        </div>
      </div>
      ${barHtml}
      ${renderResLogsDrawer(s.id)}
      ${runningJobs.length ? `<div class="sw-status-bar"><span class="spin">${ic('refresh')}</span><span>R&eacute;ponse de l'agent en cours&hellip;</span></div>` : ''}
      <div class="sw-feed" id="swConsoleFeed">${feedHtml}</div>
      <div class="sw-field">
        <label for="swAskPrompt">Message pour ${esc(s.swarm_agent || s.label)} <span class="d-meta-pill">Ctrl+Entr&eacute;e</span></label>
        <textarea id="swAskPrompt" placeholder="Tapez votre message&hellip;"></textarea>
      </div>
      <div class="sw-toolbar">
        <input id="swAskFrom" placeholder="from (ex. mcp-control)" value="mcp-control">
        <input id="swAskModel" placeholder="mod&egrave;le (optionnel)">
        <button type="button" class="primary" data-act="swarm-send-ask" ${s.state === 'UP' ? '' : 'disabled'}>Envoyer</button>
        <button type="button" data-act="swarm-clear-feed">Effacer vue</button>
      </div>
    </div>
  </div>`;
}

function renderSwarmDetailCfg(s) {
  const agents = swarmAllAgents();
  const scopedAgents = agents.filter(a => !s.swarm_agent || (a.name || '').toLowerCase() === s.swarm_agent.toLowerCase());
  const cfgAg = swarmCfgAgent
    ? agents.find(a => (a.name || '').toLowerCase() === swarmCfgAgent.toLowerCase())
    : null;
  const activeRunner = (swarmCfgData && swarmCfgData.runner) || (cfgAg && cfgAg.runner) || 'claude';
  const filesMap = (swarmCfgData && swarmCfgData.files) || {};
  const cfgFiles = swarmAvailableCfgFiles(filesMap, activeRunner);
  if (!cfgFiles.includes(swarmCfgFile)) swarmCfgFile = cfgFiles[0] || 'prompt.md';
  const isReadOnly = Boolean((swarmCfgData && swarmCfgData.read_only) || swarmCfgFile === '.env');
  const curContent = filesMap[swarmCfgFile] != null
    ? (typeof filesMap[swarmCfgFile] === 'string' ? filesMap[swarmCfgFile] : JSON.stringify(filesMap[swarmCfgFile], null, 2))
    : '';
  const wiredSet = swarmWiredMcpSet(filesMap);
  const mcpTargets = ((swarmData && swarmData.mcp_targets) || []).filter(t => !t.swarm_kind || t.swarm_kind === 'resident');
  const mcpChecks = mcpTargets.slice(0, 24).map(t => {
    const isChecked = wiredSet.has(String(t.label || '').toLowerCase()) || wiredSet.has(String(t.id || '').toLowerCase());
    return `<label class="sw-mcp-chip"><input type="checkbox" class="sw-mcp-cb" value="${esc(t.label)}"${isChecked ? ' checked' : ''}> ${esc(t.label)}</label>`;
  }).join('');

  const cfgCls = (cfgAg && cfgAg.class) || s.swarm_class || '';
  return `<div class="sw-detail-pane">
    <div class="sw-console-box">
      <div class="sw-sec-head">
        <h2>${swarmIconSvg('ic-hornet', swarmCfgAgent || s.swarm_agent || s.label, 'resident', cfgCls)} Configuration &mdash; ${esc(swarmCfgAgent || s.swarm_agent || s.label)}${swarmCfgData && swarmCfgData.read_only ? ' <span class="d-meta-pill">lecture seule (via r&eacute;sident)</span>' : ''}</h2>
        <div class="sw-toolbar">
          ${renderClassSelect(s.swarm_agent || swarmCfgAgent || '', s.id, cfgCls)}
          ${agents.length ? `<select id="swCfgAgentSel" aria-label="Agent &agrave; configurer" data-act="swarm-cfg-pick">
            <option value="">Choisir un agent&hellip;</option>
            ${(scopedAgents.length ? scopedAgents : agents).map(a => `<option value="${esc(a._instanceId)}|${esc(a.name)}|${esc(a.runner || '')}"${swarmCfgAgent === a.name ? ' selected' : ''}>${esc(a.name)} (${esc(a.runner || '?')})</option>`).join('')}
          </select>` : ''}
          <button type="button" data-act="swarm-refresh">${ic('refresh')} Actualiser</button>
        </div>
      </div>
      <div class="sw-tabs">
        ${cfgFiles.map(f => `<button type="button" class="sw-tab ${swarmCfgFile===f?'sel':''}" data-act="swarm-cfg-file" data-file="${esc(f)}">${esc(f)}</button>`).join('')}
      </div>
      <div class="sw-field">
        <label for="swCfgEditor">${esc(swarmCfgFile)}${activeRunner === 'hermes' && swarmCfgFile === 'prompt.md' ? ' (SOUL.md)' : (activeRunner === 'hermes' && swarmCfgFile === 'settings.json' ? ' (config.yaml)' : '')}</label>
        <textarea id="swCfgEditor" placeholder="Contenu de ${esc(swarmCfgFile)}&hellip;" ${isReadOnly ? 'disabled' : ''}>${esc(curContent)}</textarea>
      </div>
      <div class="sw-toolbar">
        <button type="button" class="primary" data-act="swarm-cfg-save" ${(!swarmCfgAgent || isReadOnly) ? 'disabled' : ''}>Enregistrer ${esc(swarmCfgFile)}</button>
      </div>
      ${(mcpTargets.length && activeRunner !== 'antigravity' && !(swarmCfgData && swarmCfgData.read_only)) ? `<div class="sw-field">
        <label>Serveurs MCP Control c&acirc;bl&eacute;s dans <code>mcpServers</code></label>
        <div class="sw-mcp-list">${mcpChecks}</div>
        <div><button type="button" data-act="swarm-cfg-wire-mcp" ${swarmCfgAgent ? '' : 'disabled'}>Appliquer les serveurs coch&eacute;s</button></div>
      </div>` : ''}
    </div>
  </div>`;
}

/* ===== Handlers d'actions Swarm (appelés via la table ACT de app.js) ===== */

function swarmOpenConsole(target, agentName, instId) {
  if (instId) swarmSelectedInst = instId;
  if (target) swarmConsoleTarget = target;
  if (activeView !== 'swarm') {
    openSwarmView('console', target || undefined);
  } else {
    swarmTab = 'console';
    renderSwarmView();
    if (swarmConsoleTarget.startsWith('res:')) {
      swarmEnsureResidentHistory(swarmConsoleTarget.slice(4));
    }
  }
  if (agentName && instId) {
    swarmLoadConfig(instId, agentName, '');
  }
}

function swarmInstChange(el) {
  swarmSelectedInst = el.value || '';
  renderSwarmView();
}

function swarmFilterChange() {
  const r = document.getElementById('swRunFilter');
  const st = document.getElementById('swStateFilter');
  const q = document.getElementById('swAgentSearch');
  if (r) swarmFilterRunner = r.value;
  if (st) swarmFilterState = st.value;
  if (q) swarmSearch = q.value;
  renderSwarmView();
}

function swarmToggleCreate() {
  swarmCreateOpen = !swarmCreateOpen;
  renderSwarmView();
}

async function swarmCreateSubmit() {
  const insts = (swarmData && swarmData.instances) || [];
  const upInst = insts.find(i => i.id === swarmSelectedInst && i.state === 'UP') || insts.find(i => i.state === 'UP') || insts[0];
  const inst = upInst ? upInst.id : '';
  const name = (document.getElementById('swNewName')?.value || '').trim();
  const agentCls = document.getElementById('swNewClass')?.value || swarmNewClass || 'coder';
  const runner = document.getElementById('swNewRunner')?.value || 'claude';
  const model = (document.getElementById('swNewModel')?.value || '').trim();
  const cloneVal = (document.getElementById('swNewClone')?.value || '').trim();
  const prompt = (document.getElementById('swNewPrompt')?.value || '').trim();
  const resident = !!document.getElementById('swNewResident')?.checked;
  const executable = !!document.getElementById('swNewExe')?.checked;
  if (!inst) { toast('Aucune instance Swarm UP disponible', 'err', 3000); return; }
  if (!name || !prompt) { toast('Nom et prompt syst\u00e8me requis', 'err', 3000); return; }
  const payload = {action: 'swarm_create_agent', instance_id: inst, name, runner, prompt, resident, executable, class: agentCls};
  if (model) payload.model = model;
  if (cloneVal) {
    if (runner === 'hermes') payload.cloneFrom = cloneVal;
    else payload.copyEnvFrom = cloneVal;
  }
  const j = await postAction(payload);
  if (j && j.ok) {
    toast(`Agent \u00ab ${name} \u00bb cr\u00e9\u00e9${j.followed_id ? ' et suivi en r\u00e9sident' : ''}`, 'ok', 2600);
    swarmCreateOpen = false;
    await load();
    await swarmRefresh(false);
  }
}

async function swarmAgentStart(instId, name, runner) {
  const payload = {action: 'swarm_agent_start', instance_id: instId, name, follow: true};
  if (runner) payload.runner = runner;
  const j = await postAction(payload);
  if (j && j.ok) {
    toast(`Agent r\u00e9sident \u00ab ${name} \u00bb d\u00e9marr\u00e9`, 'ok', 2500);
    await load();
    await swarmRefresh(false);
  }
}

async function swarmAgentStop(instId, name, runner) {
  const payload = {action: 'swarm_agent_stop', instance_id: instId, name};
  if (runner) payload.runner = runner;
  const j = await postAction(payload);
  if (j && j.ok) {
    toast(`Agent r\u00e9sident \u00ab ${name} \u00bb arr\u00eat\u00e9`, 'ok', 2500);
    await load();
    await swarmRefresh(false);
  }
}

async function swarmAgentDelete(instId, name, runner) {
  const ok = await askConfirm({
    title: `Supprimer l'agent \u00ab ${name} \u00bb ?`,
    msg: `Voulez-vous supprimer d\u00e9finitivement l'agent \u00ab ${name} \u00bb ?`,
    detail: 'Son dossier de configuration dans Swarm-control et son entr\u00e9e r\u00e9sidente locale seront supprim\u00e9s.',
    okLabel: 'Supprimer',
  });
  if (!ok) return;
  const payload = {action: 'swarm_delete_agent', instance_id: instId, name};
  if (runner) payload.runner = runner;
  const j = await postAction(payload);
  if (j && j.ok) {
    if (swarmCfgAgent === name) { swarmCfgAgent = ''; swarmCfgData = null; }
    toast(`Agent \u00ab ${name} \u00bb supprim\u00e9`, 'ok', 2500);
    await load();
    await swarmRefresh(false);
  }
}

async function swarmResStatus(serverId) {
  const j = await postAction({action: 'swarm_resident_status', server_id: serverId});
  if (j && j.ok) {
    const st = j.data || {};
    swarmResidentLive[serverId] = st;
    toast(`Statut : ${st.state || 'ok'} (file ${st.queueLength || 0}, tours ${st.turnsDone || 0})`, 'ok', 3000);
    await swarmRefresh(false);
  }
}

async function swarmResStop(serverId, clearQueue) {
  const j = await postAction({action: 'swarm_resident_stop', server_id: serverId, clearQueue: !!clearQueue});
  if (j && j.ok) {
    toast(clearQueue ? 'Tour arr\u00eat\u00e9 et file vid\u00e9e' : 'Tour courant arr\u00eat\u00e9', 'ok', 2500);
    await swarmRefresh(false);
  }
}

async function swarmBridgeOp(port, op) {
  const j = await postAction({action: 'swarm_bridge_action', port: Number(port), op});
  if (j && j.ok) {
    toast(`Bridge :${port} (${op}) OK`, 'ok', 2500);
  }
}

function swarmConsoleTargetChange(el) {
  swarmConsoleTarget = el.value || '';
  swarmRerenderActive();
  if (swarmConsoleTarget.startsWith('res:')) {
    swarmEnsureResidentHistory(swarmConsoleTarget.slice(4));
  }
}

async function swarmSendAsk() {
  const promptEl = document.getElementById('swAskPrompt');
  const prompt = (promptEl?.value || '').trim();
  const from = (document.getElementById('swAskFrom')?.value || '').trim() || 'mcp-control';
  const model = (document.getElementById('swAskModel')?.value || '').trim();
  const workPath = (document.getElementById('swAskPath')?.value || '').trim();
  if (!prompt) { toast('Prompt vide', 'err', 2200); return; }
  if (!swarmConsoleTarget) { toast('Aucune cible s\u00e9lectionn\u00e9e', 'err', 2500); return; }

  swarmFeedFor(swarmConsoleTarget).push({ts: Date.now(), role: 'user', title: from, meta: swarmConsoleTarget, text: prompt, err: false});
  if (promptEl) promptEl.value = '';
  swarmRerenderActive();

  if (swarmConsoleTarget.startsWith('res:')) {
    const sid = swarmConsoleTarget.slice(4);
    const payload = {action: 'swarm_resident_ask', server_id: sid, prompt, from};
    if (model) payload.model = model;
    const j = await postAction(payload);
    if (j && j.ok) {
      swarmEnsurePoll();
      await swarmPollJobs();
    }
  } else if (swarmConsoleTarget.startsWith('inst:')) {
    const parts = swarmConsoleTarget.split(':');
    const instId = parts[1];
    const agentName = parts.slice(2).join(':');
    const ag = swarmAllAgents().find(a => a._instanceId === instId && a.name === agentName);
    const payload = {
      action: 'swarm_run_agent',
      instance_id: instId,
      runner: (ag && ag.runner) || 'claude',
      agentName,
      prompt,
      background: false,
    };
    if (model) payload.model = model;
    if (workPath) payload.path = workPath;
    const j = await postAction(payload);
    if (j && j.ok) {
      swarmEnsurePoll();
      await swarmPollJobs();
    }
  }
}

function swarmClearFeed() {
  swarmFeedsByTarget[swarmConsoleTarget || '_default'] = [];
  swarmRerenderActive();
}

async function swarmLoadConfig(instId, name, runner, stayInDetail) {
  swarmSelectedInst = instId || swarmSelectedInst;
  swarmCfgAgent = name;
  if (!stayInDetail && activeView === 'swarm') swarmTab = 'console';
  const payload = {action: 'swarm_get_configs', instance_id: swarmSelectedInst, name};
  if (runner) payload.runner = runner;
  const j = await postAction(payload, {quiet: !!stayInDetail});
  if (j && j.ok) {
    swarmCfgData = Object.assign({runner: runner || undefined}, j.data || {});
    swarmRerenderActive();
  }
}

function swarmCfgPick(el) {
  const v = el.value || '';
  if (!v) return;
  const [instId, name, runner] = v.split('|');
  swarmLoadConfig(instId, name, runner, activeView !== 'swarm');
}

function swarmCfgFileSwitch(file) {
  swarmCfgFile = file || 'prompt.md';
  swarmRerenderActive();
}

async function swarmCfgSave() {
  if (!swarmCfgAgent || !swarmSelectedInst) return;
  const content = document.getElementById('swCfgEditor')?.value ?? '';
  const j = await postAction({
    action: 'swarm_update_config',
    instance_id: swarmSelectedInst,
    name: swarmCfgAgent,
    file: swarmCfgFile,
    content,
  });
  if (j && j.ok) {
    toast(`${swarmCfgFile} enregistr\u00e9 pour ${swarmCfgAgent}`, 'ok', 2500);
    await swarmLoadConfig(swarmSelectedInst, swarmCfgAgent, '', activeView !== 'swarm');
  }
}

async function swarmCfgWireMcp() {
  if (!swarmCfgAgent || !swarmSelectedInst) return;
  const checked = [...document.querySelectorAll('.sw-mcp-cb:checked')].map(el => el.value);
  const j = await postAction({
    action: 'swarm_update_config',
    instance_id: swarmSelectedInst,
    name: swarmCfgAgent,
    mcpServers: checked,
  });
  if (j && j.ok) {
    toast(`mcpServers mis \u00e0 jour (${checked.length} serveur${checked.length > 1 ? 's' : ''})`, 'ok', 2500);
    await swarmLoadConfig(swarmSelectedInst, swarmCfgAgent, '', activeView !== 'swarm');
    await swarmRefresh(false);
  }
}

function swarmSyncParallelDraftFromDom() {
  const runners = document.querySelectorAll('.sw-par-runner');
  const agents = document.querySelectorAll('.sw-par-agent');
  const prompts = document.querySelectorAll('.sw-par-prompt');
  if (!runners.length) return;
  swarmParallelDraft = [...runners].map((r, i) => ({
    runner: r.value || 'claude',
    agentName: agents[i] ? agents[i].value.trim() : '',
    prompt: prompts[i] ? prompts[i].value : '',
  }));
}

function swarmParAdd() {
  swarmSyncParallelDraftFromDom();
  if (swarmParallelDraft.length >= 10) { toast('Maximum 10 t\u00e2ches en parall\u00e8le', 'err', 2500); return; }
  swarmParallelDraft.push({runner: 'claude', agentName: '', prompt: ''});
  renderSwarmView();
}

function swarmParDel(idx) {
  swarmSyncParallelDraftFromDom();
  if (swarmParallelDraft.length <= 1) return;
  swarmParallelDraft.splice(Number(idx), 1);
  renderSwarmView();
}

async function swarmParRun() {
  swarmSyncParallelDraftFromDom();
  const insts = (swarmData && swarmData.instances) || [];
  const inst = document.getElementById('swParInst')?.value || swarmSelectedInst || (insts.find(i => i.state === 'UP') || insts[0] || {}).id || '';
  if (!inst) { toast('Aucune instance Swarm disponible', 'err', 3000); return; }
  const tasks = swarmParallelDraft
    .filter(t => t.prompt && t.prompt.trim())
    .map(t => {
      const o = {runner: t.runner || 'claude', prompt: t.prompt.trim()};
      if (t.agentName) o.agentName = t.agentName;
      return o;
    });
  if (!tasks.length) { toast('Saisissez au moins un prompt', 'err', 2500); return; }
  const waitAll = !!document.getElementById('swParWait')?.checked;
  const j = await postAction({action: 'swarm_run_parallel', instance_id: inst, agents: tasks, waitAll});
  if (j && j.ok) {
    toast(`${tasks.length} agent(s) lanc\u00e9(s) en parall\u00e8le (${j.job_id})`, 'ok', 2800);
    swarmEnsurePoll();
    await swarmPollJobs();
  }
}

async function swarmRunCtrl(instId, op, runId, agentName) {
  const payload = {action: 'swarm_agent_control', instance_id: instId, op};
  if (runId) payload.runId = runId;
  if (agentName) payload.agentName = agentName;
  const j = await postAction(payload);
  if (j && j.ok) {
    if (op === 'stream') {
      swarmStreamOut = {
        runId,
        agentName,
        text: typeof j.data === 'string' ? j.data : JSON.stringify(j.data, null, 2),
      };
      renderSwarmView();
    } else if (op === 'kill') {
      toast('Run interrompu', 'ok', 2200);
      await swarmRefresh(false);
    }
  }
}

function swarmStreamClose() {
  swarmStreamOut = null;
  renderSwarmView();
}

function swarmToggleLive() {
  swarmLive = !swarmLive;
  renderSwarmView();
}

async function swarmSyncLogos() {
  toast('Synchronisation des logos Swarm\u2026', 'ok', 1800);
  const j = await postAction({action: 'swarm_sync_logos'});
  if (j && j.ok) {
    const msg = j.updated > 0
      ? `Logos mis \u00e0 jour : ${j.updated} (${j.unchanged} inchang\u00e9s)`
      : `Logos d\u00e9j\u00e0 \u00e0 jour (${j.unchanged} v\u00e9rifi\u00e9s)`;
    toast(msg, 'ok', 2800);
    await load();
    swarmRerenderActive();
  }
}

function swarmSetA2aFilter(mode) {
  swarmA2aFilter = mode || 'all';
  renderSwarmView();
}

async function swarmSendA2a() {
  const from = (document.getElementById('swA2aFrom')?.value || '').trim() || 'mcp-control';
  const toId = document.getElementById('swA2aTo')?.value || '';
  const viaRaw = (document.getElementById('swA2aVia')?.value || '').trim();
  const promptEl = document.getElementById('swA2aPrompt');
  const prompt = (promptEl?.value || '').trim();
  if (!toId) { toast('Aucun agent r\u00e9sident UP s\u00e9lectionn\u00e9', 'err', 2500); return; }
  if (!prompt) { toast('Message A2A vide', 'err', 2200); return; }
  const via = viaRaw ? viaRaw.split(',').map(s => s.trim()).filter(Boolean) : [];
  const resObj = ((swarmData && swarmData.residents) || []).find(r => r.id === toId);
  const toName = (resObj && (resObj.agent || resObj.label)) || toId;

  swarmA2aEvents.unshift({
    ts: Date.now(),
    kind: 'a2a',
    from,
    to: toName,
    via,
    meta: 'envoy\u00e9',
    body: prompt,
    err: false,
  });
  if (swarmA2aEvents.length > 60) swarmA2aEvents.length = 60;
  if (promptEl) promptEl.value = '';
  renderSwarmView();

  const payload = {action: 'swarm_resident_ask', server_id: toId, prompt, from};
  if (via.length) payload.via = via;
  const j = await postAction(payload);
  if (j && j.ok) {
    toast(`Message A2A envoy\u00e9 vers ${toName}`, 'ok', 2200);
    swarmEnsurePoll();
    await swarmPollJobs();
  }
}

document.addEventListener('keydown', (ev) => {
  const t = ev.target;
  if (!t || !t.id) return;
  if (t.id === 'swA2aPrompt' && ev.key === 'Enter' && !ev.shiftKey) {
    ev.preventDefault();
    swarmSendA2a();
  } else if (t.id === 'swAskPrompt' && ev.key === 'Enter' && (ev.ctrlKey || ev.metaKey)) {
    ev.preventDefault();
    swarmSendAsk();
  }
});
