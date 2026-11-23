/* Code d'accès API : demandé à l'utilisateur (dialogue de déverrouillage) puis
   gardé en localStorage — JAMAIS injecté par le serveur dans la page (audit
   2026-10-24 : quiconque joint le port lisait le code sur / et contournait
   toute l'auth). En bind loopback aucun code n'est requis : le dialogue
   n'apparaît jamais. Chaque requête /api/ présente le code (X-MCC-Access) ;
   un 401 rouvre le dialogue (code tourné ou faux). */
const accessCode = () => { try { return localStorage.getItem('mcc.access') || ''; } catch (e) { return ''; } };
const MCC_VERSION = '__MCC_VERSION__';

/* ===== Thème sombre/clair (⚙ Réglages) =====
   Préférence locale (mcc.theme) appliquée AVANT le premier rendu par le
   script inline anti-flash du <body> — ici on bascule et on marque l'actif. */
// Applique le thème SANS toucher à la préférence enregistrée.
// applyTheme est aussi appelé à l'ouverture des Réglages (pour aligner les
// boutons) : écrire ici écrasait le choix de l'utilisateur — le thème clair
// ne tenait jamais (il revenait à « sombre » à la réouverture des Réglages).
function applyTheme(t) {
  document.documentElement.dataset.theme = t === 'light' ? 'light' : 'dark';
  const light = document.documentElement.dataset.theme === 'light';
  const dk = document.getElementById('tDark');
  const lt = document.getElementById('tLight');
  if (dk) dk.classList.toggle('sel', !light);
  if (lt) lt.classList.toggle('sel', light);
}
// Choix EXPLICITE de l'utilisateur : applique ET enregistre.
function applyThemeAndSave(t) {
  applyTheme(t);
  try {
    localStorage.setItem('mcc.theme', document.documentElement.dataset.theme);
    localStorage.setItem('mcc-theme', document.documentElement.dataset.theme);
  } catch (e) {}
}

/* ===== Modale de confirmation accessible (<dialog id="confirmDlg">) =====
   Remplace tous les alert(), confirm() et prompt() natifs : promesse async,
   focus sur Annuler par défaut, restauration du focus à la fermeture. */
let _confirmResolve = null;
let _lastFocusEl = null;
function rememberFocus() {
  if (document.activeElement && document.activeElement !== document.body) {
    _lastFocusEl = document.activeElement;
  }
}
function restoreFocus() {
  if (_lastFocusEl && typeof _lastFocusEl.focus === 'function') {
    try { _lastFocusEl.focus(); } catch (e) {}
  }
  _lastFocusEl = null;
}
function askConfirm(opts) {
  const o = typeof opts === 'string' ? {msg: opts} : (opts || {});
  const dlg = document.getElementById('confirmDlg');
  if (!dlg) return Promise.resolve(false);
  rememberFocus();
  document.getElementById('confirmDlgTitle').textContent = o.title || 'Confirmation';
  document.getElementById('confirmDlgMsg').textContent = o.msg || '';
  const det = document.getElementById('confirmDlgDetail');
  if (o.detail) { det.textContent = o.detail; det.hidden = false; }
  else { det.textContent = ''; det.hidden = true; }
  const inpBox = document.getElementById('confirmDlgInputs');
  if (Array.isArray(o.inputs) && o.inputs.length) {
    inpBox.hidden = false;
    inpBox.innerHTML = o.inputs.map(f =>
      `<div><label for="cdlg_${esc(f.id)}">${esc(f.label)}</label><input id="cdlg_${esc(f.id)}" type="${esc(f.type || 'text')}" placeholder="${esc(f.placeholder || '')}" value="${esc(f.value || '')}"></div>`
    ).join('');
  } else {
    inpBox.hidden = true;
    inpBox.innerHTML = '';
  }
  const btnCancel = document.getElementById('confirmDlgCancel');
  const btnOk = document.getElementById('confirmDlgOk');
  btnCancel.textContent = o.cancelLabel || 'Annuler';
  btnOk.textContent = o.okLabel || 'Confirmer';
  btnOk.classList.toggle('danger', !!o.danger);
  btnOk.classList.toggle('primary', !o.danger);
  if (!dlg.open) dlg.showModal();
  if (Array.isArray(o.inputs) && o.inputs.length) {
    const first = document.getElementById('cdlg_' + o.inputs[0].id);
    if (first) first.focus();
  } else {
    btnCancel.focus();
  }
  return new Promise(resolve => { _confirmResolve = {resolve, inputs: o.inputs || null}; });
}
function finishConfirmDlg(accepted) {
  const dlg = document.getElementById('confirmDlg');
  const pending = _confirmResolve;
  _confirmResolve = null;
  let val = accepted;
  if (accepted && pending && pending.inputs) {
    val = {};
    pending.inputs.forEach(f => {
      const el = document.getElementById('cdlg_' + f.id);
      val[f.id] = el ? el.value.trim() : '';
    });
  }
  if (dlg && dlg.open) dlg.close();
  restoreFocus();
  if (pending) pending.resolve(val);
}
function confirmDlgOk() {
  finishConfirmDlg(true);
}
function confirmDlgCancel() {
  finishConfirmDlg(false);
}
const _mccFetch = window.fetch.bind(window);

/* Session locale (docs/auth.md §6, lot S1) : le jeton d'amorçage arrive dans le
   FRAGMENT de l'URL — jamais envoyé au serveur, jamais dans Referer, jamais
   dans mcc.log. On l'échange UNE fois contre le cookie `mcc_session` ET un
   secret de session gardé UNIQUEMENT en mémoire (jamais cookie, jamais
   localStorage/sessionStorage) : il repart dans `X-MCC-Session` à chaque appel
   /api/*. Le moteur exige les deux (cookie + secret). Puis on efface le
   fragment : l'URL redevient celle qu'on peut copier sans rien livrer. */
let sessionSecret = '';
let sessionLost = false;
const SESSION_LOST_MSG = 'session expirée — rouvre le lien de démarrage (MCP Control)';
async function exchangeSession() {
  let token = '';
  try {
    const m = /[#&]t=([^&\s]+)/.exec(window.location.hash || '');
    if (m) token = decodeURIComponent(m[1]);
  } catch (e) { token = ''; }
  if (!token) return false;
  let ok = false;
  try {
    const r = await _mccFetch('/api/session', {
      method: 'POST',
      headers: {'Content-Type': 'application/json'},
      body: JSON.stringify({token: token})
    });
    const j = await r.json().catch(() => null);
    if (r.ok && j && typeof j.secret === 'string' && j.secret) {
      sessionSecret = j.secret; // mémoire seulement : jamais cookie ni stockage
      ok = true;
    }
  } catch (e) { ok = false; }
  // fragment effacé DANS TOUS LES CAS : le jeton est à usage unique, le garder
  // dans la barre d'adresse ne ferait que l'afficher.
  try { history.replaceState(null, '', window.location.pathname + window.location.search); } catch (e) {}
  return ok;
}
/* Une seule tentative par chargement ; toutes les requêtes /api/* attendent
   son issue (le cookie doit exister avant la première lecture d'état). */
const sessionReady = exchangeSession();

window.fetch = async (url, opts = {}) => {
  const isApi = typeof url === 'string' && url.startsWith('/api/');
  // Hors /api/* : requête transmise TELLE QUELLE. Indispensable pour l'IPC
  // Tauri (POST http://ipc.localhost/…) dont les en-têtes arrivent en objet
  // Headers — les recopier via Object.assign les perdait (Tauri-Invoke-Key
  // absent → 500, plus aucun drag / réduire / agrandir / fermer).
  if (!isApi) return _mccFetch(url, opts);
  await sessionReady;
  // Session perdue : plus AUCUNE requête /api/* ne part (U-9) — sans cela le
  // polleur rejouerait un 401 à chaque cycle. Réponse synthétique locale.
  if (sessionLost) {
    return new Response(JSON.stringify({ok: false, error: SESSION_LOST_MSG}),
      {status: 401, headers: {'Content-Type': 'application/json'}});
  }
  // new Headers() accepte objet simple, tableau ou Headers sans rien perdre.
  const h = new Headers(opts.headers || {});
  const code = accessCode();
  if (code) h.set('X-MCC-Access', code);
  if (sessionSecret) h.set('X-MCC-Session', sessionSecret);
  const r = await _mccFetch(url, Object.assign({}, opts, {headers: h}));
  if (r.status === 401) onUnauthorized();
  return r;
};
/* 401 de session (U-9) : on n'ouvre plus le dialogue de l'ancien code d'accès.
   Un code hérité réellement stocké (bind LAN, lecture seule) garde son dialogue ;
   sinon on annonce « session expirée » UNE fois et on cesse d'appeler /api/*. */
function onUnauthorized() {
  if (accessCode()) { showLogin(); return; }
  if (sessionLost) return;
  sessionLost = true;
  toast(SESSION_LOST_MSG, 'err', 8000);
  setDead(true, SESSION_LOST_MSG);
}
function showLogin() {
  const d = document.getElementById('loginDlg');
  if (d && !d.open) { d.showModal(); document.getElementById('loginCode').focus(); }
}
async function loginSubmit(ev) {
  if (ev && ev.preventDefault) ev.preventDefault();
  const code = document.getElementById('loginCode').value.trim();
  const msg = document.getElementById('loginMsg');
  // fetch brut (pas l'intercepteur) : pas de boucle 401 pendant la vérif
  const r = await _mccFetch('/api/state', {headers: code ? {'X-MCC-Access': code} : {}});
  if (r.ok) {
    try { localStorage.setItem('mcc.access', code); } catch (e) {}
    msg.textContent = '';
    document.getElementById('loginCode').value = '';
    document.getElementById('loginDlg').close();
    updateLogoutBtn();
    load();
  } else {
    msg.textContent = '\u2717 code refus\u00e9 — voir settings.api_access du config.json du serveur';
  }
  return false;
}

// « Oublier le code » : rien ne doit rester dans le navigateur après un poste
// partagé ou une rotation de code côté serveur.
function forgetCode() {
  try { localStorage.removeItem('mcc.access'); } catch (e) {}
  const msg = document.getElementById('loginMsg');
  if (msg) msg.textContent = '\u2713 code effac\u00e9 de ce navigateur';
  updateLogoutBtn();
}

// Le bouton de déconnexion n'apparaît que si un code est stocké.
function updateLogoutBtn() {
  const b = document.getElementById('tbLogout');
  if (b) b.hidden = !accessCode();
}
function logout() {
  forgetCode();
  const d = document.getElementById('loginDlg');
  if (d) { if (!d.open) d.showModal(); }
  load();
}

let state = null;
let selectedId = null;
let detailCache = null;
let detailTab = 'outils';
// Échappement HTML de TOUT texte venu d'un client. Les caractères de
// contrôle (C0 et DEL) sont retirés : un libellé hostile ne doit ni sortir de
// son contexte ni injecter de séquence de terminal (lot M3).
const esc = s => String(s ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c])).replace(/[\u0000-\u001f\u007f]/g, '');
let overviewFilter = null; // filtre de tuile d'accueil : 'up' = ne montrer que les UP

/* ===== Icônes =============================================================
   Un seul jeu SVG inline (trait 1.5, couleur courante, zéro emoji) : rendu
   identique quelle que soit la version d'Windows et alignement typographique
   propre dans une interface monochrome. Chaque icône est inlinée par ic() —
   pas de référence à un symbole (guillemet-double suivi de dièse interdit
   dans la raw string Rust qui embarque cette page), attributs en simples
   quotes comme le reste des SVG de la page. */
const ICONS = {
  search: "<circle cx='11' cy='11' r='7'/><path d='m21 21-4.3-4.3'/>",
  globe: "<circle cx='12' cy='12' r='8.5'/><path d='M3.5 12h17M12 3.5c2.4 2.4 3.8 5.3 3.8 8.5s-1.4 6.1-3.8 8.5c-2.4-2.4-3.8-5.3-3.8-8.5s1.4-6.1 3.8-8.5Z'/>",
  monitor: "<rect x='2.5' y='4' width='19' height='13' rx='2'/><path d='M8 21h8M12 17v4'/>",
  cart: "<circle cx='9' cy='20' r='1.4'/><circle cx='17' cy='20' r='1.4'/><path d='M2.5 3.5h2.2l2.5 11.5h10.3l2-8H6'/>",
  gear: "<circle cx='12' cy='12' r='3.1'/><path d='M12 2.8v2.6M12 18.6v2.6M2.8 12h2.6M18.6 12h2.6M5.5 5.5l1.9 1.9M16.6 16.6l1.9 1.9M18.5 5.5l-1.9 1.9M7.4 16.6l-1.9 1.9'/>",
  refresh: "<path d='M21 12a9 9 0 1 1-2.6-6.4L21 8'/><path d='M21 3v5h-5'/>",
  power: "<path d='M18.4 6.6a9 9 0 1 1-12.8 0'/><path d='M12 2.5V12'/>",
  lock: "<rect x='4.5' y='10.5' width='15' height='10' rx='2'/><path d='M8 10.5V7a4 4 0 0 1 8 0v3.5'/>",
  zap: "<path d='M13 2.5 4.5 13.5H11l-1 8 8.5-11H12l1-8Z'/>",
  star: "<path d='m12 3.6 2.5 5.1 5.7.8-4.1 4 1 5.6L12 16.4l-5.1 2.7 1-5.6-4.1-4 5.7-.8Z'/>",
  eye: "<path d='M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12Z'/><circle cx='12' cy='12' r='3'/>",
  eyeOff: "<path d='M4 4l16 16'/><path d='M10.3 5.7a9.8 9.8 0 0 1 1.7-.2c6 0 9.5 6.5 9.5 6.5a17.6 17.6 0 0 1-3 3.8M6.7 6.7C4.1 8.4 2.5 12 2.5 12S6 18.5 12 18.5c1.3 0 2.5-.3 3.6-.8'/><path d='M9.9 9.9a3 3 0 0 0 4.2 4.2'/>",
  play: "<path d='M7 4.5 19 12 7 19.5Z'/>",
  stop: "<rect x='6' y='6' width='12' height='12' rx='1.5'/>",
  pause: "<path d='M9 5v14M15 5v14'/>",
  trash: "<path d='M3.5 6h17M8.5 6V4.5A1.5 1.5 0 0 1 10 3h4a1.5 1.5 0 0 1 1.5 1.5V6M19 6l-.8 13a2 2 0 0 1-2 1.9H7.8a2 2 0 0 1-2-1.9L5 6'/><path d='M10 10.5v6M14 10.5v6'/>",
  copy: "<rect x='9' y='9' width='12' height='12' rx='2'/><path d='M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1'/>",
  tag: "<path d='m3 11 8-8h9v9l-8.5 8.5a2 2 0 0 1-2.8 0L3 13.8a2 2 0 0 1 0-2.8Z'/><circle cx='16.5' cy='7.5' r='1.3'/>",
  link: "<path d='M10 13a5 5 0 0 0 7.5.5l3-3a5 5 0 0 0-7-7l-1.7 1.7'/><path d='M14 11a5 5 0 0 0-7.5-.5l-3 3a5 5 0 0 0 7 7l1.7-1.7'/>",
  terminal: "<path d='m4 17 6-5-6-5'/><path d='M12 19h8'/>",
  folder: "<path d='M3.5 7A1.5 1.5 0 0 1 5 5.5h4.2l1.8 2h9A1.5 1.5 0 0 1 21.5 9v8a1.5 1.5 0 0 1-1.5 1.5H5A1.5 1.5 0 0 1 3.5 17Z'/>",
  external: "<path d='M14 4h6v6M20 4l-9 9'/><path d='M19 14v5a1.5 1.5 0 0 1-1.5 1.5h-12A1.5 1.5 0 0 1 4 19V7.5A1.5 1.5 0 0 1 5.5 6H10'/>",
  activity: "<path d='M22 12h-4l-3 8L9 4l-3 8H2'/>",
  check: "<path d='m4.5 12.5 5 5L19.5 7'/>",
  x: "<path d='m6 6 12 12M18 6 6 18'/>",
  bell: "<path d='M6 9a6 6 0 0 1 12 0c0 5 2 6.5 2 6.5H4S6 14 6 9Z'/><path d='M10 19.5a2.2 2.2 0 0 0 4 0'/>",
  upload: "<path d='M12 15.5V4m0 0L7.5 8.5M12 4l4.5 4.5'/><path d='M4 16.5V19a1.5 1.5 0 0 0 1.5 1.5h13A1.5 1.5 0 0 0 20 19v-2.5'/>",
  download: "<path d='M12 4v11.5m0 0L7.5 11M12 15.5l4.5-4.5'/><path d='M4 16.5V19a1.5 1.5 0 0 0 1.5 1.5h13A1.5 1.5 0 0 0 20 19v-2.5'/>",
  warn: "<path d='M12 3.5 2.5 20h19L12 3.5Z'/><path d='M12 10v4.5M12 17.1v.1'/>",
  shield: "<path d='M12 3 4.5 6v5.5c0 4.4 3.1 8.3 7.5 9.5 4.4-1.2 7.5-5.1 7.5-9.5V6L12 3Z'/><path d='m8.8 11.8 2.2 2.2 4.2-4.4'/>",
  save: "<path d='M5 3.5h11L20.5 8v11a1.5 1.5 0 0 1-1.5 1.5H5A1.5 1.5 0 0 1 3.5 19V5A1.5 1.5 0 0 1 5 3.5Z'/><path d='M8 3.5V8h7V3.5M7.5 20.5v-6h9v6'/>",
};
const ic = n => `<svg class='ic' viewBox='0 0 24 24' aria-hidden='true'>${ICONS[n] || ''}</svg>`;

/* ===== Santé moteur : bandeau « injoignable » + compteurs grisés =====
   Chaque échec du poll compte ; dès 2 échecs consécutifs le bandeau apparaît
   et les compteurs se grisent — des chiffres périmés ne passent plus pour à
   jour sans signal (audit 04 #3 : catch vide). Premier chargement jamais
   réussi : même bandeau, libellé « nouvel essai » (audit 04 #2). Le retour
   est automatique : le premier état reçu lève bandeau et grisage. */
let pollFails = 0;
const DEAD_BOOT_MSG = 'moteur injoignable, nouvel essai…';
const DEAD_MSG = 'moteur injoignable';
let _lastSelectedSrvSig = '';
let _lastAccessRevision = null;
let _stateEtag = null;
let _detailEtag = null;
let _detailEtagId = null;
let _sigServers = '';
let _sigPeers = '';
let _sigAnn = '';
let _sigLog = '';
let _sigOverview = '';
let _sigDetail = '';
let _sigAccess = '';
const setHidden = (el, h) => { if (el && el.hidden !== !!h) el.hidden = !!h; };
const setText = (el, txt) => { const s = String(txt ?? ''); if (el && el.textContent !== s) el.textContent = s; };

function setDead(on, msg) {
  const b = document.getElementById('deadBanner');
  if (b) {
    setHidden(b, !on);
    if (on) setText(b, msg);
  }
  if (document.body.classList.contains('dead') !== !!on) {
    document.body.classList.toggle('dead', on);
  }
  // boot jamais réussi : la zone principale est vide — y porter le message,
  // render() l'écrasera à la reprise.
  const c = document.getElementById('content');
  if (on && !state && c && !c.firstElementChild) {
    c.innerHTML = '<div class="engine-wait">' + esc(msg) + '</div>';
  }
}
async function load() {
  if (document.hidden) return; // fenêtre masquée (tray) : polling suspendu — CPU/mémoire au repos
  try {
    const stOpts = (state && _stateEtag) ? {headers: {'If-None-Match': _stateEtag}} : {};
    const r = await fetch('/api/state', stOpts);
    if (r.status === 304) {
      pollFails = 0;
      setDead(false);
      armPolling();
    } else {
      // Un 401 (session expirée) n'est pas un état : ne jamais le rendre.
      if (!r.ok) throw new Error('HTTP ' + r.status);
      _stateEtag = r.headers.get('ETag') || null;
      state = await r.json();
      pollFails = 0;
      setDead(false);
      if (selectedId && !state.servers.find(s => s.id === selectedId)) {
        selectedId = null; detailCache = null; _lastSelectedSrvSig = ''; _detailEtag = null; _detailEtagId = null; _sigOverview = '';
      }
      render();
      const rev = state && state.access ? state.access.revision : null;
      if (rev !== _lastAccessRevision) {
        _lastAccessRevision = rev;
        const ad = document.getElementById('accessDlg');
        if (ad && ad.open) refreshAccess();
      }
    }
    if (selectedId && state) {
      const needFreshDetail = !detailCache || detailCache.id !== selectedId || _detailEtagId !== selectedId;
      if (needFreshDetail) {
        _detailEtag = null;
        _detailEtagId = selectedId;
      }
      const srvSummary = state.servers.find(s => s.id === selectedId);
      const srvSig = JSON.stringify(srvSummary || null);
      if (needFreshDetail || srvSig !== _lastSelectedSrvSig) {
        _lastSelectedSrvSig = srvSig;
        const dOpts = (!needFreshDetail && _detailEtag) ? {headers: {'If-None-Match': _detailEtag}} : {};
        const r2 = await fetch('/api/detail?id=' + encodeURIComponent(selectedId), dOpts);
        if (r2.status === 304) return;
        if (r2.ok) {
          _detailEtag = r2.headers.get('ETag') || null;
          detailCache = await r2.json();
          renderDetail();
        }
      }
    }
  } catch(e) {
    if (sessionLost) { setDead(true, SESSION_LOST_MSG); return; }
    if (document.getElementById('loginDlg')?.open) return;
    pollFails++;
    if (state === null) setDead(true, DEAD_BOOT_MSG);
    else if (pollFails >= 2) setDead(true, DEAD_MSG);
    scheduleNext(POLL_RETRY_MS);
  }
}

function render() {
  if (!state) return;
  const s = state.stats;
  const nw = document.getElementById('netWarn');
  if (nw) {
    const off = state.lan_ok === false;
    setHidden(nw, !off);
    if (off && !nw.innerHTML) nw.innerHTML = ic('warn') + '<span>R&eacute;seau local injoignable</span>';
  }
  setText(document.getElementById('stUp'), s.up);
  setText(document.getElementById('stTotal'), s.total);
  setText(document.getElementById('stTools'), s.tools);
  armPolling();
  const au = document.getElementById('stAuto');
  if (au) {
    const autoKey = state.autostart ? '1' : '0';
    if (au.dataset.stateKey !== autoKey) {
      au.dataset.stateKey = autoKey;
      au.innerHTML = ic('power') + '<span>Auto-boot ' + (state.autostart ? 'ON' : 'OFF') + '</span>';
      au.setAttribute('aria-checked', state.autostart ? 'true' : 'false');
      au.classList.toggle('auto-on', !!state.autostart);
      au.classList.toggle('auto-off', !state.autostart);
    }
  }
  const upd = document.getElementById('stUpdate');
  if (state.update_available && state.latest_version) {
    setHidden(upd, false);
    if (upd.dataset.ver !== String(state.latest_version)) {
      upd.dataset.ver = String(state.latest_version);
      upd.innerHTML = ic('download') + '<span class="tb-lbl">Mise &agrave; jour v' + esc(String(state.latest_version)) + '</span>';
      upd.title = 'Mettre à jour vers v' + state.latest_version + ' (installée : v' + MCC_VERSION
        + ') — clic : téléchargement et installation directe';
    }
  } else setHidden(upd, true);
  renderServers();
  renderPeers();
  syncAnnouncements();
  renderLog();
  accessBadge();
  if (!selectedId) renderOverview();
}

/* ===== Annonces LiteMCP détectées, non suivies (lot S4 pt 2) =====
   Lecture seule : jamais d'adoption silencieuse. L'opérateur clique « Suivre »
   pour ajouter explicitement le serveur HTTP à sa configuration. */
let unfollowedAnnouncements = [];
let lastAnnCount = -1;
async function syncAnnouncements() {
  const count = (state && Number.isFinite(state.announcements_count)) ? state.announcements_count : 0;
  if (count === 0) {
    lastAnnCount = 0;
    if (unfollowedAnnouncements.length) {
      unfollowedAnnouncements = [];
      renderAnnouncements();
      if (!selectedId) renderOverview();
    } else {
      renderAnnouncements();
    }
    return;
  }
  if (count === lastAnnCount) {
    renderAnnouncements();
    return;
  }
  lastAnnCount = count;
  const j = await postAction({action: 'litemcp_announcements'}, {quiet: true});
  if (j && j.ok && Array.isArray(j.announcements)) {
    unfollowedAnnouncements = j.announcements.filter(a => a && a.followed === false);
  } else {
    unfollowedAnnouncements = [];
  }
  renderAnnouncements();
  if (!selectedId) renderOverview();
}
function renderAnnouncements() {
  const sec = document.getElementById('annSec');
  const box = document.getElementById('annList');
  if (!sec || !box) return;
  const sig = JSON.stringify(unfollowedAnnouncements);
  if (sig === _sigAnn) return;
  _sigAnn = sig;
  if (!unfollowedAnnouncements.length) {
    setHidden(sec, true);
    box.innerHTML = '';
    return;
  }
  setHidden(sec, false);
  box.innerHTML = unfollowedAnnouncements.map(a => {
    const portTxt = a.port ? `:${esc(a.port)}` : 'port inconnu';
    const verTxt = a.version ? ` &middot; v${esc(a.version)}` : '';
    const btn = a.port
      ? `<button class="primary" data-act="follow-ann" data-name="${esc(a.name)}" data-port="${esc(a.port)}" title="Ajouter explicitement ce serveur LiteMCP &agrave; la liste suivie">${ic('check')} Suivre</button>`
      : '';
    return `<div class="ann-row">
      <div class="l1"><span class="dot PORT-ONLY"></span><span class="nm">${esc(a.name)}</span><span class="tag litemcp">LiteMCP</span></div>
      <div class="l2">127.0.0.1${portTxt}${verTxt} &mdash; d&eacute;tect&eacute;, non suivi</div>
      ${btn}
    </div>`;
  }).join('');
}
async function followAnnouncement(name, port) {
  const p = +port;
  if (!p) { toast('Port d\u2019annonce manquant', 'err'); return; }
  const j = await postAction({action: 'add', transport: 'http', label: name || ('litemcp-' + p), port: p, path: '/mcp'});
  if (j && j.ok) {
    toast('Serveur \u00ab ' + (name || p) + ' \u00bb ajout\u00e9 \u00e0 la supervision', 'ok');
    lastAnnCount = -1;
    await act('refresh', j.id);
  }
}

/* ===== Explorateur ===== */
/* Clé de groupement : 'local' (loopback de cette machine) ou host/IP de la
   machine distante (LAN/VPS). L'explorateur affiche les locaux à plat puis
   UN GROUPE REPLIABLE PAR MACHINE distante. */
/* Hôtes loopback EXPLICITES = CETTE machine : sans cela un serveur déclaré
   sur 127.0.0.1 / ::1 / localhost atterrissait sous « Machines distantes »
   et perdait son formulaire de commande de lancement (audit 04 #6). */
const isLoopbackHost = h => ['127.0.0.1', '::1', 'localhost'].includes(String(h || '').toLowerCase());
function hostOf(s) {
  if (s.transport !== 'http') return 'local';
  const h = s.transport_info.host;
  return (!h || isLoopbackHost(h)) ? 'local' : h;
}
/* Helper global : serveur par id (applySort a son propre byId local, ombre légale). */
function byId(id) {
  return state && state.servers ? state.servers.find(s => s.id === id) : null;
}
function groupKeys() {
  // hosts des serveurs distants (ordre d'affichage) + machines enregistrées
  // (ssh_hosts) même sans serveur importé — le groupe vide montre où ranger.
  const keys = [];
  state.servers.forEach(s => { const k = hostOf(s); if (k !== 'local' && !keys.includes(k)) keys.push(k); });
  (state.settings.ssh_hosts || []).forEach(h => { if (!keys.includes(h.ip)) keys.push(h.ip); });
  return keys;
}
function hybridUrl(s) {
  if (!s || !s.hybrid_path) return '';
  return window.location.origin + s.hybrid_path;
}
function srvStoreLogo(s, lg) {
  if (!s || !s.id || !s.id.startsWith('store-')) return '';
  let inner = PKG_GLYPH;
  if (storeLogos) {
    const raw = s.id.slice(6);
    const stripped = raw.replace(/^(at|gl|gh)-/, '');
    const noNum = raw.replace(/-\d+$/, '');
    const found = storeLogos[raw] || storeLogos[stripped] || storeLogos[noNum];
    if (found) {
      inner = found;
    } else {
      const sc = s.start_cmd || s.transport_info || {};
      const cmd = ((sc.program || '') + ' ' + (sc.args || []).join(' ')).toLowerCase();
      for (const [needle, lid] of PKG_LOGOS) {
        if (storeLogos[lid] && cmd.includes(needle)) { inner = storeLogos[lid]; break; }
      }
    }
  }
  return `<span class="srv-logo${lg ? ' lg' : ''}" title="Serveur install&eacute; depuis la Boutique MCP Control"><svg viewBox="0 0 24 24" aria-hidden="true">${inner}</svg></span>`;
}
function loadStoreLogos() {
  if (storeLogos) return;
  fetch('/api/store-logos').then(r => r.ok ? r.json() : null).then(j => {
    if (!j) return;
    storeLogos = j;
    _sigServers = '';
    _sigDetail = '';
    if (state) {
      renderServers();
      if (selectedId && detailCache) renderDetail();
    }
  }).catch(() => {});
}

function srvRow(s) {
  const endpoint = s.transport === 'http'
    ? (s.transport_info.host && !isLoopbackHost(s.transport_info.host)
        ? `${esc(s.transport_info.host)}:${s.transport_info.port}`
        : `:${s.transport_info.port}`)
    : esc(s.transport_info.program || 'stdio');
  const offPill = s.tools_disabled.length
    ? `<span class="cnt-off" title="${s.tools_disabled.length} outil(s) d&eacute;sactiv&eacute;(s)">&minus;${s.tools_disabled.length}</span>`
    : '';
  const latPill = (s.state === 'UP' && s.latency_ms != null)
    ? `<span class="srow-lat">${esc(String(s.latency_ms))}ms</span>`
    : '';
  const stPill = s.state !== 'UP'
    ? `<span class="st ${esc(s.state)}">${esc(s.state)}</span>`
    : '';
  const toolsTip = `${s.tools_count} outil${s.tools_count > 1 ? 's' : ''}`
    + (s.tools_disabled.length ? ` (${s.tools_disabled.length} coup\u00e9s)` : '');
  /* Ligne 1 : état + logo boutique éventuel + nom + compteur d'outils + switch ON/OFF.
     Ligne 2 : métadonnées (état si hors-ligne, port/programme, latence, badges)
     à gauche, actions rapides (favori, autoboot, play/stop) séparées à droite. */
  return `<div class="srow ${selectedId===s.id?'sel':''} ${s.enabled?'':'off'}" draggable="true"
    role="listitem" tabindex="0" aria-label="Serveur ${esc(s.label)} (${esc(s.state)})"
    data-act="select" data-id="${esc(s.id)}">
    <div class="l1">
      <span class="dot ${esc(s.state)}"></span>${srvStoreLogo(s, false)}
      <span class="nm" title="${esc(s.label)}">${esc(s.label)}</span>
      <div class="l1-right">
        <span class="cnt" title="${esc(toolsTip)}">${s.tools_count}</span>${offPill}${rowToggle(s)}
      </div>
    </div>
    <div class="l2">
      <div class="srow-meta">
        ${stPill}<span class="srow-pill">${endpoint}</span>${latPill}${langChip(s)}${frameworkBadge(s)}${lockBadge(s)}${s.transport==='stdio'?'<span class="tag hybrid" title="Pont hybride natif : Streamable HTTP (/mcp/s/...) + auth Ed25519 LiteMCP">HTTP+AUTH</span>':''}
      </div>
      <div class="srow-acts">${rowFav(s)}${rowAuto(s)}${rowCtl(s)}</div>
    </div>
  </div>`;
}
/* Replis des groupes mémorisés (localStorage) : 'local' + une clé par machine. */
function groupClosed(key) {
  try { return localStorage.getItem('mcc.group.' + key) === '1'; } catch (e) { return false; }
}
function toggleGroup(key) {
  try { localStorage.setItem('mcc.group.' + key, groupClosed(key) ? '0' : '1'); } catch (e) {}
  renderServers();
}
function groupHeadHtml(key, label, up, total, machine) {
  const tip = machine
    ? `Machine distante ${esc(label)} &mdash; cliquer pour replier/d&eacute;plier`
    : 'Serveurs MCP locaux &agrave; cette machine (loopback) &mdash; cliquer pour replier/d&eacute;plier';
  const act = machine
    ? `<button class="gact" aria-label="Oublier la machine ${esc(label)}" title="Oublier cette machine (les serveurs d&eacute;j&agrave; import&eacute;s restent)" data-act="ssh-forget" data-id="${esc(label)}">${ic('trash')}</button>` : '';
  return `<div class="grphead ${groupClosed(key)?'closed':''}" data-act="group-toggle" data-key="${esc(key)}"
    title="${tip}"><span class="gchev">&#9662;</span><span class="gname">${esc(label)}</span>${act}<span class="gcount">${up}/${total}</span></div>`;
}
/* Filtre courant de l'explorateur : texte (champ #srvFilter) + filtre de tuile
   d'accueil (overviewFilter). Renvoie true si le serveur doit être affiché. */
/* Serveurs masqués de l'affichage — VOLONTAIREMENT en mémoire seulement :
   ni config.json ni localStorage. Fermer l'app (ou recharger la page) les fait
   réapparaître, comme demandé. La supervision moteur continue de les sonder. */
const hiddenServers = new Set();
let expStateFilter = 'all';

function setExpStateFilter(mode) {
  expStateFilter = mode || 'all';
  document.querySelectorAll('#expStatePills .exp-pill').forEach(b => {
    b.classList.toggle('sel', b.dataset.mode === expStateFilter);
  });
  renderServers();
}

function srvVisible(s, q) {
  if (hiddenServers.has(s.id)) return false;
  if (overviewFilter === 'up' && s.state !== 'UP') return false;
  if (expStateFilter === 'up' && s.state !== 'UP') return false;
  if (expStateFilter === 'down' && !(s.enabled && (s.state === 'DOWN' || s.state === 'PORT-ONLY'))) return false;
  if (expStateFilter === 'locked' && !s.locked) return false;
  if (expStateFilter === 'paused' && (s.enabled && s.state !== 'DISABLED')) return false;
  if (q) {
    const hay = (s.label + ' ' + s.id + ' ' + (s.framework || '') + ' '
      + (s.transport === 'http' ? (s.transport_info.port ?? '') : '') + ' ' + (s.state || '')).toLowerCase();
    if (!hay.includes(q)) return false;
  }
  return true;
}
function renderServers() {
  const localBox = document.getElementById('srvList');
  const groupsBox = document.getElementById('remoteGroups');
  const label = document.getElementById('remoteLabel');
  const count = document.getElementById('localCount');
  const localHead = document.getElementById('localHead');
  const clr = document.getElementById('srvFilterClear');
  const qRaw = (document.getElementById('srvFilter')?.value || '').trim();
  const q = qRaw.toLowerCase();
  if (clr) setHidden(clr, !qRaw);
  const keys = groupKeys();
  const sig = JSON.stringify([
    state.servers,
    state.settings && state.settings.ssh_hosts,
    selectedId,
    overviewFilter,
    expStateFilter,
    q,
    [...hiddenServers],
    keys.map(k => [k, groupClosed(k)]),
    groupClosed('local'),
    Object.keys(FRAMEWORKS).length
  ]);
  if (sig === _sigServers) return;
  _sigServers = sig;
  if (!state.servers.length) {
    setHidden(localHead, true); setText(count, '');
    localBox.innerHTML = '<div class="empty-sec">aucun serveur — lance un scan</div>';
    setHidden(label, true); groupsBox.innerHTML = '';
    const hb = document.getElementById('hiddenBar');
    if (hb) setHidden(hb, true);
    return;
  }
  const shown = state.servers.filter(s => !hiddenServers.has(s.id));
  const localAll = shown.filter(s => hostOf(s) === 'local');
  const local = localAll.filter(s => srvVisible(s, q));
  const remotes = shown.filter(s => hostOf(s) !== 'local');
  const byKey = {};
  remotes.forEach(s => { const k = hostOf(s); (byKey[k] = byKey[k] || []).push(s); });
  const hb = document.getElementById('hiddenBar');
  if (hb) {
    const n = hiddenServers.size;
    setHidden(hb, !n);
    if (n) hb.innerHTML = ic('eyeOff') + ' ' + n + ' serveur' + (n > 1 ? 's' : '')
      + ' masqu\u00e9' + (n > 1 ? 's' : '') + ' (reviennent \u00e0 la r\u00e9ouverture) <b>afficher</b>';
  }
  const upOf = rows => rows.filter(s => s.state === 'UP').length;
  const favFirst = rows => rows.slice().sort((a, b) => ((b.favorite === true) - (a.favorite === true)));
  const noneMatch = '<div class="empty-sec">aucun serveur ne correspond au filtre</div>';
  setHidden(localHead, false);
  localHead.classList.toggle('closed', groupClosed('local'));
  localBox.classList.toggle('closed', groupClosed('local'));
  setText(count, upOf(localAll) + '/' + localAll.length);
  localBox.innerHTML = local.length ? favFirst(local).map(srvRow).join('')
    : (localAll.length ? noneMatch : '<div class="empty-sec">aucun serveur — lance un scan</div>');
  setHidden(label, !keys.length);
  groupsBox.innerHTML = keys.map(k => {
    const all = byKey[k] || [];
    const rows = all.filter(s => srvVisible(s, q));
    const body = rows.length ? favFirst(rows).map(srvRow).join('')
      : (all.length ? noneMatch
        : '<div class="empty-sec empty-indent">aucun serveur import&eacute; de cette machine</div>');
    return groupHeadHtml(k, k, upOf(all), all.length, true)
      + `<div class="grpbody mbody ${groupClosed(k)?'closed':''}">${body}</div>`;
  }).join('');
}

// Chip du langage MCP (inféré : commande stdio ou en-têtes HTTP) — monogramme coloré façon logo
const LANG_META = {
  node:   { t: 'JS',  c: '#dcc94f', n: 'Node.js / TypeScript' },
  python: { t: 'PY',  c: '#3fb950', n: 'Python' },
  rust:   { t: 'RS',  c: '#f97316', n: 'Rust' },
  go:     { t: 'GO',  c: '#00add8', n: 'Go' },
  java:   { t: 'JV',  c: '#e76f00', n: 'Java' },
  dotnet: { t: 'C#',  c: '#512bd4', n: '.NET' },
  ruby:   { t: 'RB',  c: '#cc342d', n: 'Ruby' },
  php:    { t: 'PHP', c: '#777bb4', n: 'PHP' },
};
// Glyphe Rust : « R » dans un engrenage (lecture de l'icône .rs des éditeurs).
// NB : attributs en simples quotes — guillemet-double suivi de dièse interdit
// dans ce bloc (cette séquence fermerait la raw string Rust d'INDEX_HTML).
const RUST_LOGO = "<svg class='rustlogo' viewBox='0 0 24 24' aria-hidden='true'><g stroke='#f97316' stroke-width='1.4' stroke-linecap='round' fill='none'><circle cx='12' cy='12' r='7.9'/><path d='M19.9 12h2M17.6 17.6l1.4 1.4M12 19.9v2M6.4 17.6 5 19M4.1 12h-2M6.4 6.4 5 5M12 4.1v-2M17.6 6.4 19 5'/></g><text x='12' y='16.2' text-anchor='middle' font-size='12' font-weight='800' font-family='inherit' fill='#f97316'>R</text></svg>";
/* Logos langages embarqués (marques officielles en SVG minimal) :
   JS = carré jaune officiel ECMAScript ; Python = les deux serpents (la forme
   bleue est répétée pivotée de 180°, comme le vrai logo) ; Go = tuile cyan
   officielle ; .NET = carré violet officiel ; Java = la tasse fumante ;
   Ruby = le gem facetté ; PHP = l'ellipse violette. NB : attributs en simples
   quotes — guillemet-double suivi de dièse interdit dans ce bloc (fermerait la
   raw string Rust d'INDEX_HTML). */
const PY_PATH = "M11.8 2.1c-2.6 0-4.7 1.1-4.7 3.2v2.6h4.9v.9H5.1c-2.1 0-3.8 1.6-3.8 4.4 0 2.9 1.5 4.5 3.6 4.5h2v-2.8c0-1.9 1.6-3.5 3.5-3.5h4.4c1.6 0 2.9-1.3 2.9-2.9V5.3c0-2.1-2.1-3.2-4.7-3.2h-1.2z";
const LANG_LOGOS = {
  rust: RUST_LOGO,
  node: "<svg class='langlogo' viewBox='0 0 24 24' aria-hidden='true'><rect x='1.5' y='1.5' width='21' height='21' rx='3.5' fill='#dcc94f'/><text x='12' y='16.6' text-anchor='middle' font-size='10.5' font-weight='800' font-family='inherit' fill='#221e08'>JS</text></svg>",
  python: "<svg class='langlogo' viewBox='0 0 24 24' aria-hidden='true'><g><path fill='#3776ab' d='" + PY_PATH + "'/><circle cx='9.7' cy='5' r='.95' fill='#fff'/><g transform='rotate(180 12 12)'><path fill='#ffd43b' d='" + PY_PATH + "'/><circle cx='9.7' cy='5' r='.95' fill='#fff'/></g></g></svg>",
  go: "<svg class='langlogo' viewBox='0 0 24 24' aria-hidden='true'><rect x='1.5' y='1.5' width='21' height='21' rx='3.5' fill='#00add8'/><text x='12' y='16.2' text-anchor='middle' font-size='10' font-weight='800' font-family='inherit' fill='#fff'>GO</text></svg>",
  java: "<svg class='langlogo' viewBox='0 0 24 24' aria-hidden='true'><path fill='#f89820' d='M12.4 2.4c-.6 1.4.1 2.4 1.2 3.1 1.1.8 1.7 1.8 1.3 3 1.2-.7 1.6-2.1.8-3.3-.6-1-2-1.7-3.3-2.8z'/><path fill='#f89820' d='M9.4 7c-.5 1.1.1 2 1.1 2.5.9.6 1.4 1.4 1 2.4 1.1-.6 1.5-1.8.8-2.8-.6-.9-1.8-1.4-2.9-2.1z'/><path fill='#5382a1' d='M7.2 12.1h9.3v4.7c0 2.3-1.9 4.2-4.2 4.2h-.9c-2.3 0-4.2-1.9-4.2-4.2z'/><path fill='none' stroke='#5382a1' stroke-width='1.3' stroke-linecap='round' d='M16.9 13.4h1.2c1.1 0 2 .9 2 2s-.9 2-2 2h-1.3'/></svg>",
  dotnet: "<svg class='langlogo' viewBox='0 0 24 24' aria-hidden='true'><rect x='1.5' y='1.5' width='21' height='21' rx='3.5' fill='#512bd4'/><text x='12' y='15.4' text-anchor='middle' font-size='6.4' font-weight='700' font-family='inherit' fill='#fff'>.NET</text></svg>",
  ruby: "<svg class='langlogo' viewBox='0 0 24 24' aria-hidden='true'><path fill='#cc342d' d='M7.2 4.5h9.6l3.7 4.6L12 20.5 3.5 9.1z'/><path fill='#e9655c' d='M7.2 4.5h9.6L12 9.5z'/><path fill='#a72b24' d='M3.5 9.1h17L12 20.5z' opacity='.4'/><path fill='none' stroke='#fff' stroke-opacity='.4' stroke-width='.7' d='M7.2 4.5 12 9.5l4.8-5M12 9.5v11M3.5 9.1h17'/></svg>",
  php: "<svg class='langlogo' viewBox='0 0 24 24' aria-hidden='true'><ellipse cx='12' cy='12' rx='10.6' ry='6.2' fill='#777bb4'/><text x='12' y='14.2' text-anchor='middle' font-size='6.2' font-weight='700' font-style='italic' font-family='inherit' fill='#fff'>php</text></svg>",
};
function langChip(s) {
  const logo = LANG_LOGOS[s.lang];
  if (!logo) return '<span class="langchip unk" title="Langage non d\u00e9tect\u00e9 (ni commande ni en-t\u00eates ni process)">?</span>';
  const m = LANG_META[s.lang];
  const name = m ? m.n : s.lang;
  return `<span class="rustwrap" title="Langage inféré : ${esc(name)}">${logo}</span>`;
}

// Badge du type de MCP : STDIO / HTTP / WS — trois modes, point.
// HTTP couvre le streamable JSON et les réponses encodées SSE (FastMCP).
function typeBadge(s) {
  // Trois modes affichés, point : STDIO / HTTP / WS. Le streamable JSON et les
  // réponses encodées SSE (FastMCP) sont tous les deux « HTTP » — pas de badge
  // séparé. Le framework (LiteMCP/FastMCP) a son propre badge.
  if (s.transport === 'stdio') return '<span class="tag stdio" title="Transport stdio (process local)">STDIO</span>';
  const lan = s.transport_info.host ? '\u00b7LAN' : '';
  const via = s.via_ssh ? '\u00b7SSH' : '';
  const viaTip = s.via_ssh ? ' — sondé via SSH depuis la machine (interne)' : '';
  const suffix = (lan ? ' — serveur distant (LAN)' : '') + viaTip;
  if (s.ws) return `<span class="tag ws" title="WebSocket — le serveur accepte une upgrade WS (101)${suffix}">WS${lan}${via}</span>`;
  return `<span class="tag http" title="HTTP streamable${suffix}">HTTP${lan}${via}</span>`;
}

// Badge du framework détecté par fingerprint :
// - litemcp : initialize répondu 2026-07-28 SANS négocier + sonde /health « ok » ;
// - fastmcp : réponses initialize encodées SSE + endpoint /sse legacy co-servi
//   (signature httpStream de mcp-proxy/FastMCP). La spec MCP utilisée par le
//   serveur (s.protocol) est exposée dans le tooltip et le détail.
// Table des frameworks MCP (id → {label, lang}) chargée depuis /api/frameworks :
// une seule source de vérité côté Rust, pas de liste dupliquée en JS.
let FRAMEWORKS = {};
async function loadFrameworks() {
  try {
    const r = await fetch('/api/frameworks');
    if (!r.ok) return;
    const j = await r.json();
    (j.frameworks || []).forEach(f => { FRAMEWORKS[f.id] = f; });
    if (state) { render(); if (selectedId && detailCache) renderDetail(); }
  } catch (e) { /* hors ligne : les badges afficheront l'id brut */ }
}
function frameworkBadge(s) {
  if (!s.framework) return '';
  const f = FRAMEWORKS[s.framework] || {label: s.framework, lang: s.lang || '?'};
  const lan = s.transport_info && s.transport_info.host ? ' — serveur distant (LAN)' : '';
  const tip = `Framework ${esc(f.label)} · ${esc(f.lang)}${lan}`;
  // LiteMCP est le seul badge qui garde un halo (classe .litemcp, animation glow).
  if (s.framework === 'litemcp') {
    return `<span class="tag litemcp" title="${tip}">${esc(f.label)}</span>`;
  }
  // Les autres badges prennent la couleur du LANGAGE du framework, comme les
  // puces de code : la classe .fw-<lang> porte couleur, bordure et fond (fin des
  // styles en ligne — CSP style-src 'self').
  const lk = LANG_META[f.lang] ? f.lang : (LANG_META[s.lang] ? s.lang : '');
  if (!lk) return `<span class="tag fw" title="${tip}">${esc(f.label)}</span>`;
  return `<span class="tag fw fw-${lk}" title="${tip}">${esc(f.label)}</span>`;
}

// Détecte si les variables d'environnement d'identité MCP Control sont injectées.
function hasAuthEnv(s) {
  const envs = [
    s && s.start_cmd && s.start_cmd.env,
    s && s.transport_info && s.transport_info.env,
  ];
  return envs.some(e => e && typeof e === 'object' && ('LITEMCP_TRUSTED_KEYS' in e || 'MCC_AUTH_PUBKEY' in e));
}

// Badge cadenas + badge AUTH : serveur verrouillé / appairé RFC 9421 ou env d'auth injecté.
function lockBadge(s, verbose) {
  if (!s) return '';
  let out = '';
  if (s.locked || s.paired) {
    const paired = !!s.paired;
    const tip = paired
      ? 'Auth MCP Control active (signature RFC 9421, cl\u00e9 ' + (s.auth_keyid || 'mcc-server') + ')'
      : 'Serveur verrouill\u00e9 (401 Signature) \u2014 clic droit ou bouton Appairer pour l\u2019associer \u00e0 MCP Control';
    const lbl = verbose ? (paired ? ' Appair&eacute;' : ' Verrouill&eacute;') : '';
    out += `<span class="tag lock ${paired ? 'paired' : 'unpaired'}" title="${esc(tip)}">${ic('lock')}${lbl}</span>`;
  }
  if (hasAuthEnv(s)) {
    out += `<span class="tag auth-env" title="Variables d&#39;identit&eacute; Ed25519 MCP Control (LITEMCP_TRUSTED_KEYS / MCC_AUTH_PUBKEY) inject&eacute;es au lancement">AUTH</span>`;
  }
  return out;
}

function rowFav(s) {
  const on = !!s.favorite;
  return `<button class="rowbtn fav ${on?'on':'off'}" aria-label="${on ? 'Retirer des favoris' : 'Ajouter aux favoris'} ${esc(s.label)}" title="${on
    ? 'Retirer des favoris (l\u2019\u00e9pingle maintient la ligne en haut de sa section)'
    : '\u00c9pingler en favori : la ligne passe en haut de sa section, quel que soit le tri'}"
    data-act="set-fav" data-id="${esc(s.id)}" data-on="${on ? '0' : '1'}">${ic('star')}</button>`;
}

function rowToggle(s) {
  return `<button class="sw ${s.enabled?'on':'off'}" role="switch" aria-checked="${s.enabled?'true':'false'}" aria-label="${s.enabled?'D\u00e9sactiver':'Activer'} ${esc(s.label)}" title="${s.enabled?'D\u00e9sactiver ce serveur (arr\u00eate son process)':'Activer ce serveur (le d\u00e9marre si commande connue)'}" data-act="row-act" data-op="toggle" data-id="${esc(s.id)}">${s.enabled?'ON':'OFF'}</button>`;
}

function rowCtl(s) {
  if (!s.can_start) return '';
  const up = s.state === 'UP';
  return `<button class="rowbtn" aria-label="${up?'Arr\u00eater':'D\u00e9marrer'} ${esc(s.label)}" title="${up?'Arr\u00eater':'D\u00e9marrer'} ce serveur" data-act="row-act" data-op="${up?'kill':'start'}" data-id="${esc(s.id)}">${up?ic('stop'):ic('play')}</button>`;
}

// Autoboot PAR SERVEUR : relance automatique de CE serveur au démarrage
// de MCP Control et après un crash. Bouton ⚡ visible dans la ligne et le détail.
function rowAuto(s) {
  if (!s.can_start) return '';
  const on = s.auto_restart;
  return `<button class="rowbtn auto ${on?'on':'off'}" aria-label="Autoboot ${on?'activ\u00e9':'d\u00e9sactiv\u00e9'} pour ${esc(s.label)}" title="${on
    ? 'Autoboot ACTIV\u00c9 : ce serveur est relanc\u00e9 au d\u00e9marrage et apr\u00e8s un crash \u2014 cliquer pour d\u00e9sactiver'
    : 'Autoboot D\u00c9SACTIV\u00c9 : ce serveur ne sera pas relanc\u00e9 automatiquement \u2014 cliquer pour activer'}"
    data-act="set-auto" data-id="${esc(s.id)}" data-on="${on ? '0' : '1'}">${ic('zap')}</button>`;
}

/* État des replis du détail (mémorisé entre les cycles et les sessions) */
function saveFold(el) {
  try { localStorage.setItem('mcc.fold.' + el.dataset.fold, el.open); } catch (e) {}
}

async function toggleAutostart() {
  await postAction({action: 'set_autostart', enabled: !state.autostart});
  await load();
}

/* ===== Drag & drop : réordonner les serveurs (préférence / priorité) ===== */
let dragId = null;
function rowDragStart(ev, el, id) {
  dragId = id;
  ev.dataTransfer.effectAllowed = 'move';
  try { ev.dataTransfer.setData('text/plain', id); } catch (e) {}
  el.classList.add('dragging');
}
function rowDragOver(ev, el) {
  if (!dragId) return;
  // un serveur distant ne se déplace pas dans la zone plate locale
  if (hostOf(byId(dragId)) !== 'local') return;
  ev.preventDefault();
  ev.dataTransfer.dropEffect = 'move';
  const row = el;
  if (row.dataset.id === dragId) return;
  const rect = row.getBoundingClientRect();
  const above = ev.clientY < rect.top + rect.height / 2;
  row.classList.toggle('drop-above', above);
  row.classList.toggle('drop-below', !above);
  row._dropAbove = above;
}
function rowDragLeave(ev, el) {
  el.classList.remove('drop-above', 'drop-below');
}
function rowDrop(ev, el) {
  ev.preventDefault();
  const row = el;
  row.classList.remove('drop-above', 'drop-below');
  const moved = dragId;
  dragId = null;
  if (!moved || row.dataset.id === moved) return;
  // un serveur distant ne se déplace pas dans la zone plate locale
  if (hostOf(byId(moved)) !== 'local') return;
  // le glisser-déposer reprend la main : le menu de tri revient sur « manuel »
  manualOrder = null;
  const sel = document.getElementById('sortSel');
  if (sel) sel.value = 'manuel';
  // nouvel ordre = ids visibles, la ligne déplacée réinsérée au-dessus/en dessous
  const ids = [...document.querySelectorAll('#srvList .srow')]
    .map(r => r.dataset.id).filter(x => x !== moved);
  const at = ids.indexOf(row.dataset.id);
  ids.splice(row._dropAbove ? at : at + 1, 0, moved);
  postReorder(ids);
}
/* Drop d'une ligne de serveur sur l'EN-TÊTE d'un groupe machine : re-host du
   serveur (sa clé de groupe = le host du groupe). Cas d'usage : une machine a
   changé d'IP, ou une importation a posé le mauvais host — on déplace la ligne
   vers la bonne machine sans réimporter. */
function groupDragOver(ev, el) {
  if (!dragId) return;
  ev.preventDefault();
  ev.dataTransfer.dropEffect = 'move';
  el.classList.add('drag-over-group');
}
function groupDragLeave(ev, el) {
  el.classList.remove('drag-over-group');
}
async function groupDrop(ev, el, key) {
  ev.preventDefault();
  el.classList.remove('drag-over-group');
  const moved = dragId;
  dragId = null;
  if (!moved) return;
  const s = byId(moved);
  if (!s || s.transport !== 'http') { toast('Seuls les serveurs HTTP peuvent \u00eatre h\u00e9berg\u00e9s par une machine distante.', 'err', 4000); return; }
  const ok = await askConfirm({
    title: 'D\u00e9placer vers une machine distante',
    msg: 'H\u00e9berger \u00ab ' + (s.label || moved) + ' \u00bb sur ' + key + ' ?',
    detail: 'Le host du transport HTTP deviendra ' + key + '.',
    okLabel: 'D\u00e9placer',
  });
  if (!ok) return;
  const j = await postAction({action: 'set_host', id: moved, host: key}, {quiet: true});
  if (!j.ok) toast('set_host : ' + (j.error || '\u00e9chec'), 'err', 4000);
  await load();
}

/* ===== Menu de reclassement (⇅) =====
   Réutilise l'action API « reorder » : l'ordre appliqué est persisté
   (settings.server_order) et gouverne l'affichage ET la priorité de sondage.
   « manuel » rend l'ordre posé au glisser-déposer AVANT le tri (mémorisé côté
   client au moment où un tri est appliqué — un drag le réactualise). */
let manualOrder = null;
const SORT_STATE_RANK = {UP: 0, STARTING: 1, 'PORT-ONLY': 1, PROBING: 1, DOWN: 2, DISABLED: 3};
const frCmp = (a, b) => String(a).localeCompare(String(b), 'fr', {sensitivity: 'base'});
function sortByPort(s) {
  return s.transport === 'http' ? (s.transport_info.port || 65536) : 65536;
}
function applySort(mode) {
  if (!state || !state.servers.length) return;
  const byId = Object.fromEntries(state.servers.map(s => [s.id, s]));
  if (mode === 'manuel') {
    if (manualOrder && manualOrder.length) postReorder(manualOrder.filter(id => byId[id]));
    return;
  }
  if (manualOrder === null) manualOrder = state.servers.map(s => s.id);
  // Tri à plat sur l'ordre persisté — chaque groupe (locaux, puis machines)
  // affiche ses serveurs dans cet ordre commun, jamais un tri global mélangé.
  let ids = state.servers.map(s => s.id);
  if (mode === 'az') {
    ids.sort((a, b) => frCmp(byId[a].label, byId[b].label));
  } else if (mode === 'etat') {
    ids.sort((a, b) => ((SORT_STATE_RANK[byId[a].state] ?? 9) - (SORT_STATE_RANK[byId[b].state] ?? 9))
      || frCmp(byId[a].label, byId[b].label));
  } else if (mode === 'port') {
    ids.sort((a, b) => (sortByPort(byId[a]) - sortByPort(byId[b])) || frCmp(byId[a].label, byId[b].label));
  } else if (mode === 'latence') {
    // non sondés (null) en fin, puis du plus rapide au plus lent
    ids.sort((a, b) => ((byId[a].latency_ms ?? 1e12) - (byId[b].latency_ms ?? 1e12)));
  } else if (mode === 'tools') {
    ids.sort((a, b) => ((byId[b].tools_count || 0) - (byId[a].tools_count || 0))
      || frCmp(byId[a].label, byId[b].label));
  } else if (mode === 'framework') {
    // Regroupe par framework MCP (label humain), les serveurs sans framework
    // identifiable en fin de liste. À l'intérieur d'un framework : A→Z.
    const fwKey = s => {
      const f = FRAMEWORKS[s.framework];
      return f ? f.label : (s.framework || '\uffff');
    };
    ids.sort((a, b) => frCmp(fwKey(byId[a]), fwKey(byId[b]))
      || frCmp(byId[a].label, byId[b].label));
  } else if (mode === 'transport') {
    // stdio → HTTP → WS (WS est un HTTP avec upgrade), puis A→Z.
    const tRank = s => (s.transport === 'stdio' ? 0 : (s.ws ? 2 : 1));
    ids.sort((a, b) => (tRank(byId[a]) - tRank(byId[b]))
      || frCmp(byId[a].label, byId[b].label));
  }
  postReorder(ids);
}
async function postReorder(ids) {
  await postAction({action: 'reorder', ids});
  await load();
}
function rowDragEnd(ev, el) {
  el.classList.remove('dragging');
  document.querySelectorAll('.srow').forEach(r => r.classList.remove('drop-above', 'drop-below'));
  dragId = null;
}

function renderPeers() {
  const box = document.getElementById('peerList');
  const sig = JSON.stringify(state.peers || []);
  if (sig === _sigPeers) return;
  _sigPeers = sig;
  if (!state.peers || !state.peers.length) { box.innerHTML = '<div class="empty-sec">aucun pair d&eacute;tect&eacute;</div>'; return; }
  box.innerHTML = state.peers.map(p => {
    const st = p.stats || {};
    return `<div class="peer-row">
      <div class="l1"><span class="dot UP"></span><span class="nm">${esc(p.ip)}</span></div>
      <div class="l2">UI :${esc(p.ui_port)} &mdash; ${esc(st.up ?? '?')} UP / ${esc(st.total ?? '?')} serveurs</div>
      <button data-act="import-peer" data-id="${esc(p.ip)}">Importer ses serveurs</button>
    </div>`;
  }).join('');
}

function renderLog() {
  const lb = document.getElementById('logbody');
  const sig = JSON.stringify(state.log || []);
  if (sig === _sigLog) return;
  _sigLog = sig;
  lb.innerHTML = state.log.map(l => {
    const ts = Number.isFinite(l.ts) ? l.ts : 0;
    const d = new Date(ts * 1000).toLocaleTimeString('fr-FR');
    return `<div class="logline ${esc(l.level)}"><span class="t">${esc(d)}</span>  ${esc(l.msg)}</div>`;
  }).join('');
  lb.scrollTop = lb.scrollHeight;
}

/* ===== Zone de détail ===== */
/* Tuiles cliquables : « serveurs UP » filtre l'explorateur sur les UP,
   « serveurs suivis » remet l'affichage complet, « tools actifs » applique le
   tri par nombre d'outils, « machines distantes » ouvre le dialog SSH. */
function tileFilter(mode) {
  overviewFilter = (mode === 'up' && overviewFilter !== 'up') ? 'up' : null;
  renderServers();
  if (!selectedId) renderOverview();
}
function tileSortTools() {
  const sel = document.getElementById('sortSel');
  if (sel) sel.value = 'tools';
  applySort('tools');
}
function clearFilter() {
  const inp = document.getElementById('srvFilter');
  if (inp) inp.value = '';
  renderServers();
}
function renderOverview() {
  const s = state.stats;
  const a = state.access || {};
  const pendCount = (a.pending || 0) + (a.pairs || 0);
  const tls = (state.settings && state.settings.tls) || {};
  const sessActive = state.sessions_active || 1;
  const srvs = state.servers || [];
  const downSrvs = srvs.filter(x => x.enabled && (x.state === 'DOWN' || x.state === 'PORT-ONLY'));
  const lockedSrvs = srvs.filter(x => x.locked && x.state !== 'UP');
  const recentLogs = (state.log || []).slice(0, 5);
  const sig = JSON.stringify([
    s, a, tls, sessActive, overviewFilter,
    downSrvs.map(x => [x.id, x.label, x.state, x.can_start, x.last_error]),
    lockedSrvs.map(x => [x.id, x.label, x.state]),
    unfollowedAnnouncements,
    recentLogs,
    srvs.length
  ]);
  const contentEl = document.getElementById('content');
  if (!selectedId && sig === _sigOverview && contentEl && contentEl.querySelector('.dash-head')) return;
  _sigOverview = sig;
  const tlsLabel = tls.active ? 'TLS 1.3 actif' : (tls.compiled ? 'Loopback HTTP (TLS 1.3 pr\u00eat)' : 'Loopback seul');
  const downHtml = downSrvs.length
    ? downSrvs.slice(0, 6).map(x => `<div class="dash-item">
        <div><div class="nm">${esc(x.label)}</div><div class="meta">${esc(x.state)}${x.last_error ? ' &mdash; ' + esc(String(x.last_error).slice(0, 48)) : ''}</div></div>
        <button class="primary" data-act="row-act" data-op="${x.can_start ? 'start' : 'refresh'}" data-id="${esc(x.id)}">${ic(x.can_start ? 'play' : 'refresh')} Relancer</button>
      </div>`).join('')
    : '<div class="empty-note">Tous les serveurs actifs r&eacute;pondent normalement.</div>';
  const lockedHtml = lockedSrvs.length
    ? lockedSrvs.slice(0, 6).map(x => `<div class="dash-item">
        <div><div class="nm">${esc(x.label)}</div><div class="meta">Signature RFC 9421 requise</div></div>
        <button class="primary" data-act="server-pair" data-id="${esc(x.id)}">${ic('lock')} Appairer</button>
      </div>`).join('')
    : '<div class="empty-note">Aucun serveur verrouill&eacute; en attente d&rsquo;appairage.</div>';
  const annHtml = unfollowedAnnouncements.length
    ? unfollowedAnnouncements.slice(0, 6).map(x => `<div class="dash-item">
        <div><div class="nm">${esc(x.name)}</div><div class="meta">127.0.0.1:${esc(x.port || '?')}${x.version ? ' &middot; v' + esc(x.version) : ''}</div></div>
        ${x.port ? `<button class="primary" data-act="follow-ann" data-name="${esc(x.name)}" data-port="${esc(x.port)}">${ic('check')} Suivre</button>` : ''}
      </div>`).join('')
    : '<div class="empty-note">Aucune annonce LiteMCP locale non suivie.</div>';
  const logHtml = recentLogs.length
    ? recentLogs.map(l => {
        const ts = Number.isFinite(l.ts) ? l.ts : 0;
        const d = new Date(ts * 1000).toLocaleTimeString('fr-FR');
        return `<div class="logline ${esc(l.level)}"><span class="t">${esc(d)}</span> ${esc(l.msg)}</div>`;
      }).join('')
    : '<div class="empty-note">Aucun &eacute;v&eacute;nement r&eacute;cent.</div>';
  const firstStartGuide = !srvs.length ? `
    <div class="welcome" id="welcome">
      <div class="welcome-mark">${ic('zap')}</div>
      <div class="big">Bienvenue dans MCP Control</div>
      <div class="hint">Aucun serveur MCP n'est encore configur&eacute;. Lancez un scan local, explorez la boutique ou ajoutez une machine distante :</div>
      <div class="qa">
        <button class="primary" data-act="scan">${ic('search')} Scanner les ports locaux</button>
        <button data-act="store">${ic('cart')} Ouvrir la Boutique</button>
        <button data-act="scan-net">${ic('globe')} Scanner le r&eacute;seau</button>
        <button data-act="machines">${ic('monitor')} Choisir une machine</button>
      </div>
      <div class="hint hint-gap">Le scan utilise la plage configur&eacute;e. Pour une machine du LAN : connexion SSH par cl&eacute;,
      l'app liste ses ports en &eacute;coute, y sonde les serveurs MCP et les importe ici.</div>
    </div>` : '';
  contentEl.innerHTML = `
    <div class="dash-head">
      <div>
        <h1>Tableau de bord MCP Control</h1>
        <div class="sub">Supervision des serveurs MCP, passerelle sign&eacute;e Ed25519 et centre de s&eacute;curit&eacute; local</div>
      </div>
      <div class="d-actions">
        <button data-act="tile" data-mode="all" class="${overviewFilter ? '' : 'primary'}">Tous les serveurs (${s.total})</button>
        <button data-act="machines">${ic('monitor')} Machines SSH (${(state.settings.ssh_hosts || []).length})</button>
      </div>
    </div>
    <div class="tiles">
      <button class="tile tile-ok${overviewFilter === 'up' ? ' sel' : ''}" data-act="tile" data-mode="up"
        title="Cliquer : ne montrer que les serveurs UP dans l'explorateur (re-cliquer pour tout afficher)">
        <div class="tile-top"><div class="n ok">${s.up} / ${s.total}</div><span class="tile-ic">${ic('activity')}</span></div>
        <div class="lbl">Serveurs UP / Total</div>
        <div class="meta">${downSrvs.length ? downSrvs.length + ' en panne' : '100 % op\u00e9rationnels'}</div>
      </button>
      <button class="tile tile-acc" data-act="tile-tools"
        title="Cliquer : trier l'explorateur par nombre d'outils expos&eacute;s">
        <div class="tile-top"><div class="n">${s.tools}</div><span class="tile-ic">${ic('terminal')}</span></div>
        <div class="lbl">Outils expos&eacute;s</div>
        <div class="meta">Passerelle POST /mcp</div>
      </button>
      <button class="tile ${pendCount > 0 ? 'tile-warn' : 'tile-ok'}" data-act="access-open" data-pending="1"
        title="Cliquer : ouvrir le centre Acc&egrave;s &amp; S&eacute;curit&eacute;">
        <div class="tile-top"><div class="n ${pendCount > 0 ? 'warn' : 'ok'}">${pendCount}</div><span class="tile-ic">${ic('shield')}</span></div>
        <div class="lbl">Confirmations &amp; Appairages</div>
        <div class="meta">${a.pending || 0} ex&eacute;cution(s) &middot; ${a.pairs || 0} appairage(s)</div>
      </button>
      <button class="tile tile-acc" data-act="access-open"
        title="Cliquer : inspecter l'identit&eacute; serveur, les sessions locales et TLS 1.3">
        <div class="tile-top"><div class="n">${sessActive}</div><span class="tile-ic">${ic('lock')}</span></div>
        <div class="lbl">Sessions locales &amp; TLS</div>
        <div class="meta">${esc(tlsLabel)}</div>
      </button>
    </div>
    ${firstStartGuide}
    <div class="dash-grid">
      <div class="dash-card">
        <h3><span>${ic('warn')} Serveurs en panne</span><span class="dash-cnt">${downSrvs.length}</span></h3>
        ${downHtml}
      </div>
      <div class="dash-card">
        <h3><span>${ic('lock')} Serveurs verrouill&eacute;s</span><span class="dash-cnt">${lockedSrvs.length}</span></h3>
        ${lockedHtml}
      </div>
      <div class="dash-card">
        <h3><span>${ic('activity')} D&eacute;tect&eacute;s, non suivis</span><span class="dash-cnt">${unfollowedAnnouncements.length}</span></h3>
        ${annHtml}
      </div>
      <div class="dash-card">
        <h3><span>${ic('terminal')} Derniers &eacute;v&eacute;nements</span><span class="dash-cnt">${recentLogs.length}</span></h3>
        ${logHtml}
      </div>
    </div>`;
}

/* ===== Onglet Conf agent : prêt à coller dans Hermes / Claude / etc. ===== */
const srvName = s => (s.label || s.id || 'serveur').toLowerCase().replace(/[^a-z0-9_-]+/g, '-').replace(/^-+|-+$/g, '') || 'serveur';
/* URL http d'un transport : un hôte IPv6 (il contient « : ») doit être entouré
   de crochets — http://[::1]:4104/mcp et non http://::1:4104/mcp (invalide,
   ambigu). Un seul helper pour tous les blocs d'URL (audit 04 #5). */
const hostForUrl = h => { const s = String(h || ''); return s.includes(':') ? '[' + s + ']' : s; };
const srvUrl = s => {
  const h = s.transport_info.host || '127.0.0.1';
  return 'http://' + hostForUrl(h) + ':' + s.transport_info.port + (s.transport_info.path || '/mcp');
};
function hermesYaml(s) {
  const n = srvName(s);
  if (s.transport === 'http')
    return 'mcp_servers:\n  ' + n + ':\n    url: ' + srvUrl(s) + '\n    enabled: true';
  const args = (s.transport_info.args || []).map(a => JSON.stringify(a)).join(', ');
  return 'mcp_servers:\n  ' + n + ':\n    command: ' + (s.transport_info.program || 'npx') + '\n    args: [' + args + ']\n    enabled: true';
}
function mcpJson(s) {
  const n = srvName(s);
  const blk = s.transport === 'http'
    ? {type: 'http', url: srvUrl(s)}
    : {command: s.transport_info.program || 'npx', args: s.transport_info.args || []};
  return JSON.stringify({mcpServers: {[n]: blk}}, null, 2);
}
function claudeCmd(s) {
  const n = srvName(s);
  return s.transport === 'http'
    ? 'claude mcp add --transport http ' + n + ' ' + srvUrl(s)
    : 'claude mcp add ' + n + ' -- ' + (s.transport_info.program || 'npx') + ' ' + (s.transport_info.args || []).join(' ');
}
function copyConf(btn) {
  const pre = btn.closest('.conf-block').querySelector('pre');
  copyText(pre.textContent); // repli execCommand intégré (audit 04 #11)
  btn.classList.add('copied');
  btn.innerHTML = ic('check') + '<span>Copi&eacute;</span>';
  setTimeout(() => {
    btn.classList.remove('copied');
    btn.innerHTML = ic('copy') + '<span>Copier</span>';
  }, 1500);
}
function hlYaml(src) {
  return String(src || '').split('\n').map(line => {
    const m = line.match(/^(\s*)([A-Za-z0-9_.-]+)(:)(\s*)(.*)$/);
    if (!m) return esc(line);
    const [, ind, key, col, sp, val] = m;
    let vHtml = esc(val);
    if (/^(true|false|null|\d+)$/.test(val)) vHtml = '<span class="tok-b">' + esc(val) + '</span>';
    else if (val) vHtml = '<span class="tok-s">' + esc(val) + '</span>';
    return esc(ind) + '<span class="tok-k">' + esc(key) + '</span><span class="tok-p">' + esc(col) + '</span>' + esc(sp) + vHtml;
  }).join('\n');
}
function hlJson(src) {
  return esc(src)
    .replace(/&quot;([^&]*?)&quot;(\s*:)/g, '<span class="tok-k">&quot;$1&quot;</span><span class="tok-p">$2</span>')
    .replace(/(:\s*)&quot;([^&]*?)&quot;/g, '$1<span class="tok-s">&quot;$2&quot;</span>')
    .replace(/(&quot;[^&]*?&quot;)/g, '<span class="tok-s">$1</span>')
    .replace(/\b(true|false|null)\b/g, '<span class="tok-b">$1</span>');
}
function hlCli(src) {
  return String(src || '').split(' ').map((p, idx) => {
    if (idx === 0) return '<span class="tok-c">' + esc(p) + '</span>';
    if (p.startsWith('-')) return '<span class="tok-b">' + esc(p) + '</span>';
    if (/^https?:\/\//.test(p)) return '<span class="tok-s">' + esc(p) + '</span>';
    return esc(p);
  }).join(' ');
}
function confAgent(s) {
  const blk = (t, lang, html) =>
    '<div class="conf-block"><div class="conf-head"><span class="conf-lang">' + esc(lang) + '</span><span>' + t +
    '</span><button class="copy-btn" data-act="copy-conf">' + ic('copy') + '<span>Copier</span></button></div><pre>' + html + '</pre></div>';
  let hybridBlk = '';
  const hUrl = hybridUrl(s);
  if (hUrl) {
    const hJson = JSON.stringify({
      mcpServers: {
        [s.id]: {
          command: 'mcp-control',
          args: ['bridge', '--url', hUrl],
        },
      },
    }, null, 2);
    hybridBlk = blk(
      'Pont hybride Streamable HTTP + Auth LiteMCP Ed25519 (' + esc(hUrl) + ')',
      'JSON',
      hlJson(hJson)
    );
  }
  return '<h4 class="sec">Connecter ce serveur &agrave; un agent</h4>' +
    '<div class="conf-note">Copie le bloc de ton agent &mdash; Hermes : colle sous <b>mcp_servers:</b> dans <b>~/.hermes/config.yaml</b>. Cl&eacute;s API : bloc <b>env:</b> (Hermes) ou <b>"env"</b> (JSON) &mdash; voir la desc du serveur.</div>' +
    hybridBlk +
    blk('Hermes &mdash; config.yaml', 'YAML', hlYaml(hermesYaml(s))) +
    blk('JSON mcpServers (Claude, Cursor, VS Code&hellip;)', 'JSON', hlJson(mcpJson(s))) +
    blk('claude mcp add', 'CLI', hlCli(claudeCmd(s)));
}
let toolSearchQuery = '';
const expandedTools = new Set();

function mdInline(t) {
  return esc(t)
    .replace(/\*\*([^*]+)\*\*/g, '<b>$1</b>')
    .replace(/(^|[^*])\*([^*\s][^*]*)\*/g, '$1<i>$2</i>')
    .replace(/`([^`]+)`/g, '<code>$1</code>');
}
const toolArgsDraft = new Map();
const toolTestState = new Map();

function buildArgsTemplate(schema) {
  if (!schema || typeof schema !== 'object' || !schema.properties || typeof schema.properties !== 'object') {
    return {};
  }
  const out = {};
  for (const [k, def] of Object.entries(schema.properties)) {
    if (!def || typeof def !== 'object') { out[k] = ''; continue; }
    if (Object.prototype.hasOwnProperty.call(def, 'default')) { out[k] = def.default; continue; }
    if (Array.isArray(def.enum) && def.enum.length) { out[k] = def.enum[0]; continue; }
    const ty = Array.isArray(def.type) ? def.type[0] : def.type;
    if (ty === 'number' || ty === 'integer') out[k] = 0;
    else if (ty === 'boolean') out[k] = false;
    else if (ty === 'array') out[k] = [];
    else if (ty === 'object') out[k] = {};
    else out[k] = '';
  }
  return out;
}
function formatSchemaType(def) {
  if (!def || typeof def !== 'object') return 'any';
  if (Array.isArray(def.type)) return def.type.join(' | ');
  if (def.type === 'array' && def.items && def.items.type) return 'array<' + def.items.type + '>';
  if (def.type) return String(def.type);
  if (Array.isArray(def.enum)) return 'enum';
  if (Array.isArray(def.anyOf)) return def.anyOf.map(formatSchemaType).join(' | ');
  return 'any';
}
function renderToolParamsHtml(schema) {
  const props = schema && typeof schema === 'object' && schema.properties && typeof schema.properties === 'object'
    ? Object.entries(schema.properties) : [];
  if (!props.length) {
    return '<div class="empty-note">Aucun param&egrave;tre d&eacute;clar&eacute; dans le sch&eacute;ma (objet vide <code>{}</code>).</div>';
  }
  const req = new Set(Array.isArray(schema.required) ? schema.required : []);
  const rows = props.map(([name, def]) => {
    const isReq = req.has(name);
    const ty = formatSchemaType(def);
    const dHtml = def && def.description ? mdInline(def.description) : '<span class="t3">&mdash;</span>';
    const chips = [];
    if (def && Array.isArray(def.enum) && def.enum.length) {
      chips.push('<span class="param-chip">enum: ' + esc(def.enum.map(String).join(' | ')) + '</span>');
    }
    if (def && Object.prototype.hasOwnProperty.call(def, 'default')) {
      chips.push('<span class="param-chip">d&eacute;faut: ' + esc(JSON.stringify(def.default)) + '</span>');
    }
    return `<tr>
      <td><code>${esc(name)}</code>${isReq ? '<span class="param-req">requis</span>' : ''}</td>
      <td class="param-type">${esc(ty)}</td>
      <td>${dHtml}${chips.length ? '<div class="param-meta">' + chips.join('') + '</div>' : ''}</td>
    </tr>`;
  }).join('');
  return `<table class="tool-params"><thead><tr><th>Param&egrave;tre</th><th>Type</th><th>Description</th></tr></thead><tbody>${rows}</tbody></table>`;
}
function renderToolPanelHtml(s, t, expKey) {
  const schema = t.input_schema || null;
  const schemaJson = schema ? JSON.stringify(schema, null, 2) : null;
  const canRun = s.state === 'UP' && (s.transport === 'http' || s.transport === 'stdio') && !s.via_ssh && (!s.locked || s.paired) && !t.disabled;
  const draft = toolArgsDraft.has(expKey)
    ? toolArgsDraft.get(expKey)
    : JSON.stringify(buildArgsTemplate(schema), null, 2);
  const st = toolTestState.get(expKey) || null;
  let resHtml = '';
  if (st) {
    if (st.loading) {
      resHtml = '<div class="tool-res"><div class="tool-res-head"><span>Ex&eacute;cution en cours&hellip;</span></div></div>';
    } else if (st.error) {
      resHtml = `<div class="tool-res"><div class="tool-res-head"><span class="tool-res-err">Erreur</span></div><pre>${esc(st.error)}</pre></div>`;
    } else if (st.result !== undefined) {
      const isErr = !!(st.result && st.result.isError);
      const badge = isErr ? '<span class="tool-res-err">isError: true</span>' : '<span class="tool-res-ok">OK</span>';
      const ms = st.elapsed_ms != null ? `<span>${esc(String(st.elapsed_ms))} ms</span>` : '';
      const raw = JSON.stringify(st.result, null, 2);
      resHtml = `<div class="tool-res"><div class="tool-res-head">${badge}${ms}</div><pre>${hlJson(raw)}</pre></div>`;
    }
  }
  const testerHtml = canRun ? `
    <div class="tool-sec-h"><span>Tester l'outil en direct</span></div>
    <textarea class="tool-args-ta" rows="5" spellcheck="false" aria-label="Arguments JSON pour ${esc(t.name)}"
      data-act="tool-args-input" data-id="${esc(s.id)}" data-tool="${esc(t.name)}">${esc(draft)}</textarea>
    <div class="tool-actions-bar">
      <button type="button" class="primary" data-act="tool-run" data-id="${esc(s.id)}" data-tool="${esc(t.name)}">${ic('play')} Ex&eacute;cuter</button>
      <button type="button" data-act="tool-args-reset" data-id="${esc(s.id)}" data-tool="${esc(t.name)}">R&eacute;initialiser</button>
    </div>
    ${resHtml}` : `
    <div class="empty-note">${t.disabled ? 'Outil d&eacute;sactiv&eacute; &mdash; r&eacute;active-le pour le tester.'
      : s.transport !== 'http' ? 'Serveur stdio : ex&eacute;cution directe HTTP uniquement (copie le payload <code>tools/call</code> ci-dessus).'
      : s.via_ssh ? 'Serveur distant via SSH : ex&eacute;cution directe non support&eacute;e.'
      : s.locked ? 'Serveur verrouill&eacute; : appaire-le d\'abord pour tester ses outils.'
      : 'Serveur hors ligne (UP requis pour ex&eacute;cuter un outil).'}</div>`;
  return `<div class="tool-panel">
    <div class="tool-full-desc">${t.desc ? mdInline(t.desc) : '<span class="t3">Aucune description fournie par le serveur.</span>'}</div>
    <div class="tool-sec-h"><span>Param&egrave;tres</span></div>
    ${renderToolParamsHtml(schema)}
    <div class="conf-block">
      <div class="conf-head">
        <span class="conf-lang">JSON Schema</span>
        <span>inputSchema</span>
        <button type="button" class="copy-btn" data-act="copy-tool-rpc" data-tool="${esc(t.name)}">${ic('copy')}<span>Copier tools/call</span></button>
        <button type="button" class="copy-btn" data-act="copy-tool-schema" data-tool="${esc(t.name)}">${ic('copy')}<span>Copier sch&eacute;ma</span></button>
      </div>
      <pre>${schemaJson ? hlJson(schemaJson) : '<span class="tok-p">// Aucun inputSchema retourn&eacute; par ce serveur — clique sur « Sonder » pour rafra&icirc;chir</span>'}</pre>
    </div>
    ${testerHtml}
  </div>`;
}
function renderToolsRowsHtml(s) {
  const all = s.tools_list || [];
  const q = toolSearchQuery.trim().toLowerCase();
  const filtered = q ? all.filter(t => (t.name + ' ' + (t.desc || '')).toLowerCase().includes(q)) : all;
  if (!filtered.length) {
    return `<div class="empty-note">${all.length ? 'aucun outil ne correspond au filtre' : 'aucun tool list&eacute;'}</div>`;
  }
  return filtered.map(t => {
    const expKey = s.id + '::' + t.name;
    const isExp = expandedTools.has(expKey);
    const propsCount = t.input_schema && t.input_schema.properties && typeof t.input_schema.properties === 'object'
      ? Object.keys(t.input_schema.properties).length : 0;
    const pBadge = propsCount > 0 ? `<span class="tool-pbadge">${propsCount} param.</span>` : '';
    return `<div class="tool ${t.disabled?'off':''} ${isExp?'expanded':''}" data-tool="${esc(t.name)}">
       <div class="tool-head" data-act="tool-expand" data-tool="${esc(t.name)}" title="Cliquer pour inspecter le sch&eacute;ma et tester l'outil">
         <span class="tool-chev" aria-hidden="true">&#9656;</span>
         <code>${esc(t.name)}</code>
         <span class="desc">${t.desc ? mdInline(t.desc) : '&mdash;'}</span>
         ${pBadge}
         <button class="tgl ${t.disabled?'off':'on'}" role="switch" aria-checked="${t.disabled?'false':'true'}" aria-label="${t.disabled?'R&eacute;activer':'D&eacute;sactiver'} l'outil ${esc(t.name)}" title="${t.disabled?'R&eacute;activer ce tool':'D&eacute;sactiver ce tool'}"
                 data-act="tool-toggle" data-id="${esc(s.id)}" data-tool="${esc(t.name)}">
           ${t.disabled ? 'OFF' : 'ON'}
         </button>
       </div>
       ${isExp ? renderToolPanelHtml(s, t, expKey) : ''}
     </div>`;
  }).join('');
}
function refreshToolsBox() {
  const box = document.getElementById('toolsRowsBox');
  if (box && detailCache) box.innerHTML = renderToolsRowsHtml(detailCache);
}
function setToolSearch(el) {
  toolSearchQuery = el ? el.value : '';
  refreshToolsBox();
}
function toggleToolExpand(el) {
  if (!detailCache || !el || !el.dataset.tool) return;
  const expKey = detailCache.id + '::' + el.dataset.tool;
  if (expandedTools.has(expKey)) expandedTools.delete(expKey);
  else expandedTools.add(expKey);
  refreshToolsBox();
}
function copyToolSchema(toolName) {
  if (!detailCache || !toolName) return;
  const t = (detailCache.tools_list || []).find(x => x.name === toolName);
  const txt = t && t.input_schema ? JSON.stringify(t.input_schema, null, 2) : '{}';
  copyText(txt);
  toast('Sch\u00e9ma JSON copi\u00e9', 'ok', 1800);
}
function copyToolRpc(toolName) {
  if (!detailCache || !toolName) return;
  const t = (detailCache.tools_list || []).find(x => x.name === toolName);
  const expKey = detailCache.id + '::' + toolName;
  let args = buildArgsTemplate(t && t.input_schema);
  if (toolArgsDraft.has(expKey)) {
    try { args = JSON.parse(toolArgsDraft.get(expKey)); } catch (e) {}
  }
  const rpc = {jsonrpc: '2.0', id: 1, method: 'tools/call', params: {name: toolName, arguments: args}};
  copyText(JSON.stringify(rpc, null, 2));
  toast('Payload tools/call copi\u00e9', 'ok', 1800);
}
function saveToolArgsDraft(el) {
  if (!el || !el.dataset.id || !el.dataset.tool) return;
  toolArgsDraft.set(el.dataset.id + '::' + el.dataset.tool, el.value);
}
function resetToolArgsDraft(id, toolName) {
  if (!detailCache || !id || !toolName) return;
  const expKey = id + '::' + toolName;
  const t = (detailCache.tools_list || []).find(x => x.name === toolName);
  toolArgsDraft.set(expKey, JSON.stringify(buildArgsTemplate(t && t.input_schema), null, 2));
  toolTestState.delete(expKey);
  refreshToolsBox();
}
async function runToolTest(id, toolName) {
  if (!id || !toolName) return;
  const expKey = id + '::' + toolName;
  const raw = toolArgsDraft.has(expKey)
    ? toolArgsDraft.get(expKey)
    : (() => {
        const t = detailCache && (detailCache.tools_list || []).find(x => x.name === toolName);
        return JSON.stringify(buildArgsTemplate(t && t.input_schema), null, 2);
      })();
  let parsed;
  try {
    parsed = raw.trim() ? JSON.parse(raw) : {};
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
      throw new Error('Les arguments doivent \u00eatre un objet JSON { ... }.');
    }
  } catch (e) {
    toolTestState.set(expKey, {error: 'JSON invalide : ' + e.message});
    refreshToolsBox();
    return;
  }
  toolTestState.set(expKey, {loading: true});
  refreshToolsBox();
  const j = await postAction({action: 'call_tool', id, tool: toolName, arguments: parsed}, {quiet: true});
  if (j.ok) {
    toolTestState.set(expKey, {result: j.result, elapsed_ms: j.elapsed_ms});
  } else {
    toolTestState.set(expKey, {error: j.error || 'erreur lors de l\u2019appel'});
  }
  refreshToolsBox();
}
async function setAllTools(id, enableAll) {
  if (!detailCache || detailCache.id !== id) return;
  const list = (detailCache.tools_list || []).filter(t => enableAll ? t.disabled : !t.disabled);
  if (!list.length) return;
  await Promise.all(list.map(t => postAction({action: 'toggle_tool', id, tool: t.name}, {quiet: true})));
  const r = await fetch('/api/detail?id=' + encodeURIComponent(id));
  if (r.ok) { detailCache = await r.json(); renderDetail(); }
  await load();
}

function renderDetail() {
  const s = detailCache;
  if (!s || s.id !== selectedId) return;
  const aEl = document.activeElement;
  if (aEl && (aEl.id === 'scProgram' || aEl.id === 'scArgs' || aEl.id === 'scWorkdir' || aEl.id === 'toolSearch'
    || (aEl.closest && aEl.closest('#toolsRowsBox')))) return;
  const sig = JSON.stringify([s, detailTab, toolSearchQuery, Object.keys(FRAMEWORKS).length]);
  const contentEl = document.getElementById('content');
  if (selectedId && sig === _sigDetail && contentEl && contentEl.querySelector('.d-head')) return;
  _sigDetail = sig;
  const typed = {};
  ['scProgram', 'scArgs', 'scWorkdir'].forEach(idOld => {
    const el = document.getElementById(idOld);
    if (el && el.value) typed[idOld] = el.value;
  });
  const ti = s.transport === 'http'
    ? `HTTP ${s.transport_info.host ? s.transport_info.host + ':' : ''}${s.transport_info.port}${s.transport_info.path}`
    : `stdio \u00b7 ${s.transport_info.program} ${(s.transport_info.args || []).join(' ')}`;
  const kvRow = (k, v) => {
    const cpy = v && v !== '—';
    return `<div class="k">${k}</div><div class="v${cpy ? ' cpy' : ''}"${cpy
      ? ' title="Cliquer pour copier dans le presse-papiers" data-act="copy-self"' : ''}>${esc(v)}</div>`;
  };
  const kv = [
    ['id', s.id], ['transport', ti],
    ['origine', s.origin],
    ['bind', s.bound_host ?? '—'], ['serveur MCP', s.server_name ?? '—'],
    ['version', s.version ?? '—'], ['protocole', s.protocol ?? '—'],
    ['framework', s.framework
      ? ((FRAMEWORKS[s.framework] || {label: s.framework, lang: '?'}).label
         + ' (' + (FRAMEWORKS[s.framework] || {lang: s.lang || '?'}).lang
         + ' · spec ' + (s.protocol || '?') + ')')
      : '—'],
    ['sonde', s.via_ssh ? 'via SSH (interne à la machine)' : 'directe'],
    ['latence', s.latency_ms != null ? s.latency_ms + ' ms' : '—'],
    ['dernier vu', s.last_seen ? new Date(s.last_seen * 1000).toLocaleString('fr-FR') : '—'],
    ['erreur', s.last_error ?? '—'],
  ].map(([k, v]) => kvRow(k, v)).join('');
  const starting = s.enabled && s.managed && s.state !== 'UP';
  const dispState = starting ? 'STARTING' : s.state;
  const etatProcess = s.state === 'UP' ? 'en marche'
    : starting ? 'au d\u00e9marrage\u2026'
    : s.state === 'PORT-ONLY' ? 'port occup\u00e9 (aucune r\u00e9ponse MCP)'
    : s.state === 'DISABLED' ? 'd\u00e9sactiv\u00e9' : 'arr\u00eat\u00e9';
  const cmd = s.start_cmd
    ? s.start_cmd.program + ' ' + (s.start_cmd.args || []).join(' ') + (s.start_cmd.workdir ? '  \u00b7  ' + s.start_cmd.workdir : '')
    : null;
  const kvd = [
    ['contr&ocirc;le', s.managed ? 'process lanc\u00e9 et maintenu par MCP Control'
      : (cmd ? 'commande pr\u00eate \u2014 process externe non d\u00e9marr\u00e9 par ici' : 'aucune \u2014 process externe')],
    ['&eacute;tat du process', etatProcess],
    ['pid', s.pid ?? '—'],
    ['auto-restart', s.auto_restart ? 'ON \u2014 relance automatique au boot/crash' : 'OFF'],
    ['commande', cmd ?? '—'],
  ].map(([k, v]) => kvRow(k, v)).join('');
  const foldOpen = name => {
    try {
      const v = localStorage.getItem('mcc.fold.' + name);
      if (v !== null) return v === 'true';
    } catch (e) {}
    return name === 'demarrage';
  };
  const fold = (name, title, inner) =>
    `<details class="fold" data-fold="${name}"${foldOpen(name) ? ' open' : ''}><summary>${title}</summary>${inner}</details>`;
  const nOn = (s.tools_list || []).filter(t => !t.disabled).length;
  const nTot = (s.tools_list || []).length;
  const toolsHtml = `<h4 class="sec">Outils &mdash; ${nOn} actif${nOn > 1 ? 's' : ''} sur ${nTot}${nTot - nOn ? ` (${nTot - nOn} d&eacute;sactiv&eacute;${nTot - nOn > 1 ? 's' : ''})` : ''}</h4>
    ${nTot > 0 ? `<div class="tools-bar">
      <label for="toolSearch" class="sr-only">Rechercher un outil</label>
      <input id="toolSearch" placeholder="Filtrer les outils de ce serveur&hellip;" data-act="tool-search" value="${esc(toolSearchQuery)}">
      <button data-act="tools-all-on" data-id="${esc(s.id)}" ${nOn === nTot ? 'disabled' : ''}>${ic('check')} Tout activer</button>
      <button data-act="tools-all-off" data-id="${esc(s.id)}" ${nOn === 0 ? 'disabled' : ''}>${ic('x')} Tout couper</button>
    </div>` : ''}
    <div id="toolsRowsBox">${renderToolsRowsHtml(s)}</div>`;
  const endpointLabel = s.transport === 'http'
    ? ((s.transport_info.host || '127.0.0.1') + ':' + s.transport_info.port + (s.transport_info.path || '/mcp'))
    : (hybridUrl(s) || ('stdio · ' + (s.transport_info.program || '')));
  const needsPair = s.transport === 'http' && s.locked && !s.paired;
  contentEl.innerHTML = `
    <div class="crumb">serveurs / <b>${esc(s.label)}</b></div>
    <div class="d-head st-${esc(dispState)}">
      <div class="d-head-top">
        <span class="dot dot-lg ${esc(dispState)}"></span>${srvStoreLogo(s, true)}
        <h1>${esc(s.label)}</h1>
        <span class="pill ${esc(dispState)}" ${starting ? 'title="Process lanc&eacute;, MCP pas encore r&eacute;pondu"' : ''}>${starting ? 'AU D&Eacute;MARRAGE' : esc(s.state)}</span>
        <span class="d-head-endpoint" title="${esc(endpointLabel)}">${esc(endpointLabel)}</span>
      </div>
      <div class="d-meta-strip">
        ${langChip(s)}
        ${typeBadge(s)}
        ${frameworkBadge(s)}
        ${lockBadge(s, true)}
        ${s.transport === 'stdio' ? '<span class="tag hybrid" title="Pont hybride natif MCP Control : Streamable HTTP POST ' + esc(s.hybrid_path || '') + ' prot&eacute;g&eacute; par signature Ed25519 LiteMCP">HYBRIDE &middot; HTTP + AUTH</span>' : ''}
        ${s.protocol ? `<span class="d-meta-pill">MCP ${esc(s.protocol)}</span>` : ''}
        ${s.server_name ? `<span class="d-meta-pill">${esc(s.server_name)}${s.version ? ' v' + esc(s.version) : ''}</span>` : ''}
        ${s.latency_ms != null ? `<span class="d-meta-pill">${esc(String(s.latency_ms))} ms</span>` : ''}
        ${s.pid ? `<span class="d-meta-pill">PID ${esc(String(s.pid))}</span>` : ''}
      </div>
      <div class="d-actions">
        <div class="d-act-grp">
          ${needsPair ? `<button class="primary" data-act="server-pair" data-id="${esc(s.id)}" title="Appairer MCP Control avec ce serveur verrouill&eacute; par signature RFC 9421">${ic('lock')} Appairer</button>` : ''}
          ${s.can_start && s.state !== 'UP' ? `<button class="primary" data-act="row-act" data-op="start" data-id="${esc(s.id)}">${ic('play')} D&eacute;marrer</button>` : ''}
          ${s.can_start && s.state === 'UP' ? `<button class="danger" data-act="row-act" data-op="kill" data-id="${esc(s.id)}">${ic('stop')} Arr&ecirc;ter</button>` : ''}
          <button data-act="row-act" data-op="refresh" data-id="${esc(s.id)}">${ic('activity')} Sonder</button>
        </div>
        <span class="d-act-sep" aria-hidden="true"></span>
        <div class="d-act-grp">
          ${s.can_start ? `<button class="autobtn ${s.auto_restart?'on':''}" role="switch" aria-checked="${s.auto_restart?'true':'false'}" data-act="set-auto" data-id="${esc(s.id)}" data-on="${s.auto_restart?'0':'1'}"
            title="Autoboot : relance automatique de ce serveur au d&eacute;marrage de MCP Control et apr&egrave;s un crash">${ic('zap')} Autoboot ${s.auto_restart?'ON':'OFF'}</button>` : ''}
          <button class="favbtn ${s.favorite?'on':''}" role="switch" aria-checked="${s.favorite?'true':'false'}" data-act="set-fav" data-id="${esc(s.id)}" data-on="${s.favorite?'0':'1'}"
            title="Favori : &eacute;pingle ce serveur en haut de sa section dans l'explorateur (persist&eacute;)">${ic('star')} Favori ${s.favorite?'ON':'OFF'}</button>
        </div>
        <div class="d-act-grp d-act-right">
          <button class="btn-ghost" data-act="hide-server" data-id="${esc(s.id)}" title="Masquer ce serveur de l'affichage (session seulement : il revient &agrave; la r&eacute;ouverture de l'app)">${ic('eyeOff')} Masquer</button>
          <button data-act="row-act" data-op="toggle" data-id="${esc(s.id)}" title="${s.enabled
            ? 'Mettre en pause : le process est arr&ecirc;t&eacute; et le serveur n&#39;est plus sond&eacute; (il reste dans la liste, r&eacute;activable d&#39;un clic)'
            : 'Reprendre la supervision : le serveur sera sond&eacute; et d&eacute;marr&eacute; si une commande est connue'}">${s.enabled ? ic('pause') + ' Pause' : ic('play') + ' Reprendre'}</button>
          ${s.origin !== 'seed'
            ? `<button class="danger" data-act="remove-srv" data-id="${esc(s.id)}">${ic('trash')} Supprimer</button>`
            : `<button class="danger" data-act="remove-seed" data-id="${esc(s.id)}" title="Supprime ce seed de la config : il ne sera pas r&eacute;inject&eacute; au red&eacute;marrage">${ic('trash')} Supprimer</button>`}
        </div>
      </div>
    </div>
    ${needsPair ? `<div class="d-lock-banner"><div>${ic('lock')} <b>Serveur verrouill&eacute; par signature Ed25519 (RFC 9421)</b> &mdash; associez l'identit&eacute; locale <code>mcc-server</code> pour autoriser la sonde et le proxy.</div><button class="primary" data-act="server-pair" data-id="${esc(s.id)}">${ic('lock')} Appairer ce serveur</button></div>` : ''}
    ${s.origin === 'seed' ? '<div class="seed-note">Serveur seed : <b>Retirer durablement</b> l&rsquo;efface m&ecirc;me apr&egrave;s red&eacute;marrage (consign&eacute; dans config.json) ; <b>D&eacute;sactiver</b> l&rsquo;&eacute;teint sans le supprimer.</div>' : ''}
    ${s.last_error ? `<div class="d-err">${ic('warn')} ${esc(s.last_error)}</div>` : ''}
    ${fold('demarrage', 'D&eacute;marrage', `<div class="kv">${kvd}</div>${startCmdForm(s)}`)}
    ${fold('infos', 'Informations', `<div class="kv">${kv}</div>`)}
    <div class="tabs" role="tablist" aria-label="Vues du serveur">
      <button class="tab ${detailTab === 'outils' ? 'sel' : ''}" role="tab" id="dtab-outils" aria-selected="${detailTab === 'outils' ? 'true' : 'false'}" aria-controls="dpanel-main" tabindex="${detailTab === 'outils' ? '0' : '-1'}" data-act="detail-tab" data-tab="outils">Outils (${nTot})</button>
      <button class="tab ${detailTab === 'conf' ? 'sel' : ''}" role="tab" id="dtab-conf" aria-selected="${detailTab === 'conf' ? 'true' : 'false'}" aria-controls="dpanel-main" tabindex="${detailTab === 'conf' ? '0' : '-1'}" data-act="detail-tab" data-tab="conf">Conf agent</button>
    </div>
    <div id="dpanel-main" role="tabpanel" aria-labelledby="dtab-${esc(detailTab)}">
      ${detailTab === 'conf' ? confAgent(s) : toolsHtml}
    </div>`;
  for (const idOld in typed) {
    const el = document.getElementById(idOld);
    if (el && !el.value) el.value = typed[idOld];
  }
}

function select(id) {
  selectedId = id;
  detailCache = null;
  _lastSelectedSrvSig = '';
  _sigDetail = '';
  _sigOverview = '';
  toolSearchQuery = '';
  detailTab = 'outils';
  render();
  load();
}

/* Formulaire « commande de lancement » : pour un serveur local découvert par
   scan (port connu, commande inconnue) — poser la commande fait apparaître les
   boutons ▶/■ et l'autoboot, comme un serveur ajouté à la main. */
function startCmdForm(s) {
  if (s.can_start || s.transport === 'stdio') return '';
  const host = s.transport_info && s.transport_info.host;
  if (host && !isLoopbackHost(host)) return '<div class="conf-note note-gap">Serveur distant : la commande de lancement appartient &agrave; sa machine (' + esc(host) + '), pas à MCP Control.</div>';
  const port = (s.transport_info || {}).port || 0;
  return `<div class="startcmd-form note-gap">
    <div class="conf-note">Aucune commande de lancement connue : c'est pourquoi les boutons <b>&#9654;/&#9632;</b> et <b>&#9889;</b> sont absents. Indique comment lancer ce serveur :</div>
    <div class="machine-form">
      <input id="scProgram" placeholder="programme (node, python, C:\\chemin\\serveur.exe)">
      <input id="scArgs" placeholder="arguments (ex : server.js --port ${esc(String(port))})">
      <input id="scWorkdir" placeholder="dossier de travail (optionnel)">
      <button class="primary" data-act="set-start-cmd" data-id="${esc(s.id)}">Enregistrer</button>
    </div>
  </div>`;
}
async function setStartCmd(id) {
  const program = (document.getElementById('scProgram').value || '').trim();
  const args = (document.getElementById('scArgs').value || '').split(/\s+/).filter(Boolean);
  const workdir = (document.getElementById('scWorkdir').value || '').trim();
  if (!program) { toast('Programme requis (ex : node, python, chemin .exe)', 'err', 3500); return; }
  const j = await postAction({action: 'set_start_cmd', id, program, args, workdir: workdir || null}, {quiet: true});
  if (!j.ok) toast(j.error || 'erreur', 'err', 4000);
  else { await load(); detailCache = null; if (selectedId) renderDetail(); }
}
async function removeSrv(id) {
  const s = byId(id);
  const ok = await askConfirm({
    title: 'Supprimer ce serveur',
    msg: 'Supprimer \u00ab ' + (s ? (s.label || id) : id) + ' \u00bb de la liste ?',
    detail: 'Un serveur re-d\u00e9tect\u00e9 par un scan r\u00e9appara\u00eetra.',
    okLabel: 'Supprimer',
    danger: true,
  });
  if (!ok) return;
  selectedId = null; detailCache = null; act('remove', id);
}
async function removeSeedForever(id) {
  const ok = await askConfirm({
    title: 'Retirer ce seed durablement',
    msg: 'Retirer ce seed DURABLEMENT ?',
    detail: 'Il ne sera pas r\u00e9inject\u00e9 au red\u00e9marrage de MCP Control.',
    okLabel: 'Retirer durablement',
    danger: true,
  });
  if (!ok) return;
  selectedId = null; detailCache = null;
  postAction({action: 'remove', id, durable: true}).then(() => load());
}

/* ===== Actions ===== */
/* ---- Menu contextuel (clic droit sur une ligne de serveur) ---------------- */
let ctxId = null;
/* Entrées du menu contextuel courant : les items ne portent qu'un index
   (data-i), la table ACT relit la fermeture dans ce tableau — aucun code
   n'est jamais construit depuis une valeur reçue. */
let ctxItems = [];
function ctxClose() {
  const m = document.getElementById('ctxmenu');
  if (m) { m.hidden = true; m.innerHTML = ''; }
  ctxId = null;
}
/* Toast sémantique : toast(msg) neutre, toast(msg,'ok') succès,
   toast(msg,'err') erreur (affichée plus longtemps — un échec se lit) ;
   `ms` surcharge la durée quand le message est long à lire. */
function toast(msg, kind, ms) {
  const t = document.getElementById('toast');
  if (!t) return;
  t.textContent = msg;
  t.classList.remove('ok', 'err');
  if (kind) t.classList.add(kind);
  t.classList.add('on');
  clearTimeout(toast._t);
  toast._t = setTimeout(() => t.classList.remove('on'), ms || (kind === 'err' ? 3500 : 1600));
}
/* Détail de l'alerte réseau (clic sur la pastille de la barre du Journal) :
   le message complet n'encombre pas la barre, il s'affiche à la demande. */
const NET_WARN_DETAIL = 'Réseau local injoignable : les serveurs distants ne peuvent pas être sondés'
  + ' — VPN actif ? (Proton VPN : activer «\u00a0autoriser les connexions LAN\u00a0»)';
function showNetDetail() {
  toast(NET_WARN_DETAIL, 'err', 6000);
}
function copyText(txt) {
  const done = () => toast('copié', 'ok');
  if (navigator.clipboard && navigator.clipboard.writeText) {
    navigator.clipboard.writeText(txt).then(done).catch(() => { copyFallback(txt); done(); });
  } else { copyFallback(txt); done(); }
}
function copyFallback(txt) {
  const ta = document.createElement('textarea');
  ta.value = txt; ta.className = 'copy-ta';
  document.body.appendChild(ta); ta.select();
  try { document.execCommand('copy'); } catch (e) {}
  document.body.removeChild(ta);
}
function normPath(p) { return String(p || '').replace(/\\/g, '/').replace(/\/+$/, ''); }
// Chemin absolu + relatif (au workdir) : programme du start_cmd, ou fichier de
// script passé en argument quand le programme est un lanceur nu (node, npx,
// python…). Aucun chemin inventé : si rien n'est résolvable, la valeur est vide.
function srvPaths(s) {
  const ti = s.transport_info || {}, sc = s.start_cmd || {};
  const wd = normPath(ti.workdir || sc.workdir || '');
  const prog = ti.program || sc.program || '';
  const args = ti.args || sc.args || [];
  let abs = '';
  if (s.transport === 'stdio') {
    if (prog && /[\\/]/.test(prog)) abs = prog;
    else {
      const fileArg = args.find(a => /\.(js|mjs|cjs|jsx|ts|py|exe|jar|rb|php|sh|bat|cmd)$/i.test(a));
      // à défaut, un chemin absolu passé en argument (ex. server-filesystem C:/Users/X)
      const dirArg = args.find(a => /^[A-Za-z]:[\\/]/.test(a) || /^\/(?![-])/.test(a));
      if (fileArg) abs = (wd ? wd + '/' : '') + String(fileArg).replace(/\\/g, '/').replace(/^\/+/, '');
      else if (dirArg) abs = dirArg;
    }
  } else if (prog) abs = prog;
  const a = normPath(abs);
  const rel = (a && wd && a.toLowerCase().startsWith(wd.toLowerCase() + '/')) ? a.slice(wd.length + 1) : '';
  return { abs: a, rel: rel, workdir: wd };
}
// URL d'un serveur HTTP — variante gardée (la constante srvUrl plus haut ne
// vérifie pas le transport et sert au bloc « Conf agent »).
function ctxUrl(s) {
  if (s.transport === 'stdio') return hybridUrl(s);
  if (s.transport !== 'http') return '';
  const ti = s.transport_info || {};
  return 'http://' + hostForUrl(ti.host || '127.0.0.1') + ':' + ti.port + (ti.path || '/mcp');
}
function srvCmd(s) {
  const sc = s.start_cmd || {};
  if (!sc.program) return '';
  return [sc.program].concat(sc.args || []).join(' ');
}
function srvInfoText(s) {
  const p = srvPaths(s);
  const fw = (typeof FRAMEWORKS === 'object' && FRAMEWORKS[s.framework]) ? FRAMEWORKS[s.framework].label : (s.framework || '—');
  const L = [
    'serveur      : ' + s.label,
    'id           : ' + s.id,
    'état         : ' + s.state + (s.enabled ? '' : ' (désactivé)') + (s.auto_restart ? ' · autoboot' : ''),
    'transport    : ' + s.transport + (s.ws ? ' (WS)' : ''),
    'url          : ' + (ctxUrl(s) || '—'),
    'commande     : ' + (srvCmd(s) || '—'),
    'framework    : ' + fw,
    'langage      : ' + (s.lang || '—'),
    'version      : ' + (s.version || '—') + ' · protocole ' + (s.protocol || '—'),
    'nom annoncé  : ' + (s.server_name || '—'),
    'outils       : ' + s.tools_count + '/' + s.tools_total + ((s.tools_disabled || []).length ? ' (' + s.tools_disabled.length + ' off)' : ''),
    'latence      : ' + (s.latency_ms != null ? s.latency_ms + ' ms' : '—'),
    'pid          : ' + (s.pid != null ? s.pid : '—')
  ];
  if (p.abs) L.push('chemin       : ' + p.abs);
  if (p.rel) L.push('chemin rel.  : ' + p.rel);
  if (p.workdir) L.push('workdir      : ' + p.workdir);
  if (s.last_error) L.push('dernière err : ' + s.last_error);
  return L.join('\n');
}
async function openInExplorer(path) {
  if (!path) return;
  const j = await postAction({action: 'open_path', path: path}, {quiet: true});
  toast(j.ok ? 'explorateur ouvert' : ('échec : ' + (j.error || 'inconnu')), j.ok ? 'ok' : 'err');
}
function rowCtx(ev, id) {
  ev.preventDefault();
  ev.stopPropagation();
  const s = (state && state.servers ? state.servers : []).find(x => x.id === id);
  if (!s) return;
  if (selectedId !== id) select(id);
  const p = srvPaths(s);
  const items = [];
  const add = (lbl, fn, dis) => items.push({lbl: lbl, fn: fn, dis: !!dis});
  add(ic('copy') + ' Copier toutes les infos', () => copyText(srvInfoText(s)));
  add(ic('tag') + ' Copier le nom', () => copyText(s.label));
  if (ctxUrl(s)) add(ic('link') + " Copier l'URL", () => copyText(ctxUrl(s)));
  if (srvCmd(s)) add(ic('terminal') + ' Copier la commande', () => copyText(srvCmd(s)));
  add(ic('folder') + ' Copier le chemin', () => copyText(p.abs), !p.abs);
  add(ic('folder') + ' Copier le chemin relatif', () => copyText(p.rel), !p.rel);
  add(ic('external') + " Ouvrir dans l'explorateur", () => openInExplorer(p.abs || p.workdir), !(p.abs || p.workdir));
  if (s.can_start) {
    add('—', null, false);
    add(s.state === 'UP' ? ic('stop') + ' Arrêter ce serveur' : ic('play') + ' Démarrer ce serveur',
        () => act(s.state === 'UP' ? 'kill' : 'start', s.id));
  }
  add(ic('refresh') + ' Sonder maintenant', () => act('refresh', s.id));
  // Section Auth MCP Control (RFC 9421 / Ed25519 + variables d'env)
  add('—', null, false);
  if (s.transport === 'http') {
    if (s.paired) {
      add(ic('shield') + ' Auth MCP Control active \u2713 (' + esc(s.auth_keyid || 'mcc-server') + ')', null, true);
      add(ic('copy') + ' Copier la cl\u00e9 publique appair\u00e9e', () => copyMcpIdentity(s));
      add(ic('refresh') + ' R\u00e9-appairer l\u2019auth MCP Control', () => serverRepair(s.id));
      add(ic('trash') + ' Retirer l\u2019auth de ce serveur', () => serverUnpair(s.id));
    } else {
      add(ic('shield') + ' Appairer l\u2019auth MCP Control (Ed25519)', () => serverPair(s.id));
      add(ic('copy') + ' Copier l\u2019identit\u00e9 Ed25519 (cl\u00e9 publique)', () => copyMcpIdentity(s));
    }
  } else {
    add(ic('copy') + ' Copier l\u2019identit\u00e9 Ed25519 (cl\u00e9 publique)', () => copyMcpIdentity(s));
  }
  if (s.transport === 'stdio' || s.can_start) {
    const envOn = hasAuthEnv(s);
    add(
      ic('terminal') + (envOn ? ' Retirer l\u2019env d\u2019auth MCP Control' : ' Injecter l\u2019env d\u2019auth MCP Control'),
      () => toggleServerAuthEnv(s.id, !envOn)
    );
  }
  add('—', null, false);
  add(ic('eyeOff') + ' Masquer', () => hideServer(s.id));
  showMenu(ev, items);
}
// Affiche un menu d'entrées {lbl, fn, dis} à la position du curseur (via data-x/data-y : 0 style inline).
const CTX_X_STEPS = [16,48,80,112,144,176,208,240,272,304,336,368,400,448,512,576,640,704,768,832,896,960,1024,1120,1216,1312,1408,1504,1600];
const CTX_Y_STEPS = [16,48,80,112,144,176,208,240,272,304,336,368,400,448,512,576,640,704,768,832];
function snapStep(val, steps) {
  let best = steps[0];
  for (let i = 0; i < steps.length; i++) {
    if (Math.abs(steps[i] - val) < Math.abs(best - val)) best = steps[i];
  }
  return best;
}
function showMenu(ev, items) {
  const m = document.getElementById('ctxmenu');
  ctxItems = items;
  m.innerHTML = items.map((it, i) => it.lbl === '—'
    ? '<div class="ctxsep" role="separator"></div>'
    : '<div class="ctxitem' + (it.dis ? ' dis' : '') + '" role="menuitem" tabindex="-1" data-act="ctx-item" data-i="' + i + '">' + it.lbl + '</div>').join('');
  m.hidden = false;
  const w = m.offsetWidth || 260, h = m.offsetHeight || 220;
  const x = Math.max(16, Math.min(ev.clientX || 16, window.innerWidth - w - 8));
  const y = Math.max(16, Math.min(ev.clientY || 16, window.innerHeight - h - 8));
  m.dataset.x = String(snapStep(x, CTX_X_STEPS));
  m.dataset.y = String(snapStep(y, CTX_Y_STEPS));
}
// Clic droit sur un OUTIL du panneau de détail.
function toolCtx(ev, id, name) {
  ev.preventDefault();
  ev.stopPropagation();
  // state.servers ne porte PAS tools_list : seul detailCache l'expose.
  const s = (state && state.servers ? state.servers : []).find(x => x.id === id) || {};
  const d = (detailCache && detailCache.id === id) ? detailCache : {};
  const t = (d.tools_list || []).find(x => x.name === name) || {name: name, desc: ''};
  const items = [];
  const add = (lbl, fn, dis) => items.push({lbl: lbl, fn: fn, dis: !!dis});
  add(ic('copy') + " Copier le nom de l'outil", () => copyText(name));
  add(ic('copy') + ' Copier « serveur.outil »', () => copyText((d.label || s.label || id) + '.' + name));
  if (t.desc) add(ic('copy') + ' Copier la description', () => copyText(t.desc));
  add(ic('terminal') + " Copier l'appel JSON-RPC", () => copyText(JSON.stringify({
    jsonrpc: '2.0', id: 1, method: 'tools/call',
    params: {name: name, arguments: {}}
  }, null, 2)));
  if (ctxUrl(s)) add(ic('link') + " Copier l'URL + outil", () => copyText(ctxUrl(s) + '   # outil : ' + name));
  add('—', null, false);
  add(t.disabled ? ic('check') + ' Réactiver cet outil' : ic('x') + ' Désactiver cet outil',
      () => toggleTool(id, name));
  showMenu(ev, items);
}
// Clic droit : délégation (plus fiable que des attributs inline sur du HTML
// régénéré à chaque rendu). Ligne serveur → menu serveur, ligne outil → menu outil.
document.addEventListener('contextmenu', (ev) => {  const t = ev.target && ev.target.closest ? ev.target : null;
  if (!t) return;
  const row = t.closest('.srow');
  if (row && row.dataset.id) { ev.preventDefault(); ev.stopPropagation(); rowCtx(ev, row.dataset.id); return; }
  const tool = t.closest('.tool');
  if (tool && tool.dataset.tool && selectedId) {
    ev.preventDefault(); ev.stopPropagation(); toolCtx(ev, selectedId, tool.dataset.tool);
  }
});
// Diagnostic : une exception JS non capturée s'écrit dans le TITRE de la
// fenêtre (lisible depuis l'extérieur, ex. PowerShell MainWindowTitle) — le
// webview n'a pas de console accessible sur la build packagée.
window.addEventListener('error', (e) => {
  try { document.title = 'JSERR ' + (e.message || e.error || 'inconnu'); } catch (_) {}
});
window.addEventListener('unhandledrejection', (e) => {
  try { document.title = 'JSERR promesse ' + ((e.reason && e.reason.message) || e.reason || 'inconnu'); } catch (_) {}
});
/* Délégation centrale (lot S2) : un écouteur par TYPE d'événement sur le
   document — plus aucun attribut on* dans la page. Le menu contextuel se
   ferme au clic hors de lui ; l'action du contrôle cliqué passe par la table
   ACT (plus bas). Les plis <details> n'ont pas d'événement `toggle` qui
   bouillonne : on enregistre leur état après le clic sur <summary>.
   Ordre : tous les gestionnaires sont des déclarations hoistées, et ACT
   n'est lue qu'à l'événement — après l'exécution complète du script. */
document.addEventListener('click', (ev) => {
  const t = ev.target;
  if (!t || !t.closest) return;
  if (!t.closest('#ctxmenu')) ctxClose();
  runAction(ev);
});
document.addEventListener('toggle', (ev) => {
  const d = ev.target;
  if (d && d.matches && d.matches('details.fold[data-fold]')) saveFold(d);
}, true);
document.addEventListener('change', runAction);
document.addEventListener('input', runAction);
document.addEventListener('submit', (ev) => { ev.preventDefault(); runAction(ev); });
// Glisser-déposer : les événements HTML5 bouillonnent, on résout la cible au
// plus proche porteur de la classe attendue (ligne serveur ou en-tête de groupe).
const closestOf = (ev, sel) => (ev.target && ev.target.closest ? ev.target.closest(sel) : null);
document.addEventListener('dragstart', (ev) => {
  const r = closestOf(ev, '.srow[data-id]');
  if (r) rowDragStart(ev, r, r.dataset.id);
});
document.addEventListener('dragover', (ev) => {
  const r = closestOf(ev, '.srow[data-id]');
  if (r) { rowDragOver(ev, r); return; }
  const g = closestOf(ev, '.grphead[data-key]');
  if (g) groupDragOver(ev, g);
});
document.addEventListener('dragleave', (ev) => {
  const r = closestOf(ev, '.srow[data-id]');
  if (r) rowDragLeave(ev, r);
  const g = closestOf(ev, '.grphead[data-key]');
  if (g) groupDragLeave(ev, g);
});
document.addEventListener('drop', (ev) => {
  const r = closestOf(ev, '.srow[data-id]');
  if (r) { rowDrop(ev, r); return; }
  const g = closestOf(ev, '.grphead[data-key]');
  if (g) groupDrop(ev, g, g.dataset.key);
});
document.addEventListener('dragend', (ev) => {
  const r = closestOf(ev, '.srow[data-id]');
  if (r) rowDragEnd(ev, r);
});
/* Navigation clavier complète (WCAG 2.2 AA) :
   - Flèches ←/→/↑/↓ dans role="tablist" (Détail, Réglages, Accès)
   - Flèches ↑/↓, Entrée et Alt+↑/Alt+↓ (réordonner) sur les lignes .srow de l'explorateur
   - Ctrl+B : replier / déplier l'explorateur
   - Échap : ferme menu contextuel ou efface filtre et restaure le focus */
document.addEventListener('keydown', (e) => {
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'b') {
    e.preventDefault();
    toggleExplorer();
    return;
  }
  const el = document.activeElement;
  if (el && el.getAttribute && el.getAttribute('role') === 'tab') {
    const tablist = el.closest('[role="tablist"]');
    if (tablist && (e.key === 'ArrowRight' || e.key === 'ArrowLeft' || e.key === 'ArrowDown' || e.key === 'ArrowUp' || e.key === 'Home' || e.key === 'End')) {
      const tabs = [...tablist.querySelectorAll('[role="tab"]:not([disabled])')];
      const idx = tabs.indexOf(el);
      if (idx >= 0 && tabs.length > 1) {
        e.preventDefault();
        let next = idx;
        if (e.key === 'ArrowRight' || e.key === 'ArrowDown') next = (idx + 1) % tabs.length;
        else if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') next = (idx - 1 + tabs.length) % tabs.length;
        else if (e.key === 'Home') next = 0;
        else if (e.key === 'End') next = tabs.length - 1;
        const target = tabs[next];
        target.focus();
        target.click();
        return;
      }
    }
  }
  const row = el && el.closest ? el.closest('.srow[data-id]') : null;
  if (row) {
    if (e.altKey && (e.key === 'ArrowUp' || e.key === 'ArrowDown')) {
      e.preventDefault();
      const id = row.dataset.id;
      const localRows = [...document.querySelectorAll('#srvList .srow[data-id]')];
      const ids = localRows.map(r => r.dataset.id);
      const idx = ids.indexOf(id);
      const swapIdx = e.key === 'ArrowUp' ? idx - 1 : idx + 1;
      if (idx >= 0 && swapIdx >= 0 && swapIdx < ids.length) {
        const tmp = ids[idx]; ids[idx] = ids[swapIdx]; ids[swapIdx] = tmp;
        manualOrder = null;
        const sel = document.getElementById('sortSel');
        if (sel) sel.value = 'manuel';
        postReorder(ids).then(() => {
          const movedEl = document.querySelector(`#srvList .srow[data-id="${CSS.escape(id)}"]`);
          if (movedEl) movedEl.focus();
        });
      }
      return;
    }
    if (!e.altKey && (e.key === 'ArrowUp' || e.key === 'ArrowDown')) {
      e.preventDefault();
      const allRows = [...document.querySelectorAll('#explorer .srow[data-id]')];
      const idx = allRows.indexOf(row);
      const next = e.key === 'ArrowDown' ? idx + 1 : idx - 1;
      if (next >= 0 && next < allRows.length) allRows[next].focus();
      return;
    }
    if (e.key === 'Enter' && el === row) {
      e.preventDefault();
      select(row.dataset.id);
      return;
    }
  }
  if (e.key === 'Escape') {
    ctxClose();
    const f = document.getElementById('srvFilter');
    if (f && (f.value || document.activeElement === f)) {
      f.value = '';
      renderServers();
      f.blur();
    }
    return;
  }
  const typing = el && (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA'
    || el.tagName === 'SELECT' || el.isContentEditable);
  if (!typing && (e.key === '/' || ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k'))) {
    e.preventDefault();
    const d = document.getElementById('storeDlg');
    if (d && d.open) {
      const ss = document.getElementById('storeSearch');
      if (ss) { ss.focus(); ss.select(); }
      return;
    }
    const f = document.getElementById('srvFilter');
    if (f) { f.focus(); f.select(); }
  }
});
window.addEventListener('resize', ctxClose);

/* ===== Envoi d'action — UN SEUL chemin (lot C #1) =====
   postAction(body, opts) : POST /api/action, lit j.ok, affiche j.error en
   toast d'erreur et RENVOIE la réponse (les appelants réagissent à j.added,
   j.found, j.started…). Tout envoi d'action de la page passe par ici —
   contrat testé (plus aucun fetch d'action ailleurs). Les appelants qui
   affichent déjà le refus dans leur propre zone (messages de dialog, volet
   réglages, replis de mise à jour) passent {quiet: true} pour éviter un
   double signal. */
function netDown() {
  // Panne réseau constatée (erreur de transport, pas un refus) : bandeau
  // immédiat au lieu d'attendre le prochain cycle de sondage (lot C #3), et
  // poll passé en réessai rapide. Renvoie une réponse synthétique ok:false
  // pour que les appelants réagissent (repli downloadUpdate…).
  if (!document.getElementById('loginDlg')?.open) {
    pollFails = Math.max(pollFails, 1); // le prochain échec de poll compte comme 2ᵉ
    setDead(true, state === null ? DEAD_BOOT_MSG : DEAD_MSG);
    scheduleNext(POLL_RETRY_MS);
  }
  return {ok: false, error: 'moteur injoignable', netDown: true};
}
async function postAction(body, opts) {
  let r;
  try {
    r = await fetch('/api/action', {method: 'POST',
      headers: {'Content-Type': 'application/json'},
      body: JSON.stringify(body)});
  } catch (e) {
    return netDown(); // erreur réseau : bandeau immédiat (lot C #3)
  }
  let j = null;
  try { j = await r.json(); } catch (e) { /* corps non JSON : couvert plus bas */ }
  if (!j) {
    // pas de corps JSON (ex. 401 pendant le dialog du code d'accès) : le
    // dialog couvre déjà l'écran — pas de double signal.
    if (document.getElementById('loginDlg')?.open) return {ok: false, error: "code d'accès requis"};
    j = {ok: false, error: 'réponse illisible du moteur (HTTP ' + r.status + ')'};
  }
  if (!j.ok && !(opts && opts.quiet)) toast(j.error || 'échec', 'err'); // refus moteur : silence interdit (audit 04 #7)
  return j;
}
async function act(action, id) {
  const j = await postAction({action, id});
  if (!j.ok && j.needs_force && Array.isArray(j.occupants)) {
    const desc = j.occupants.map(o => (o.process || 'processus') + ' (PID ' + o.pid + ')').join(', ');
    const force = await askConfirm({
      title: 'Processus tiers sur le port',
      msg: 'Ce port est tenu par un processus qui n\u2019a pas \u00e9t\u00e9 lanc\u00e9 par MCP Control. Voulez-vous l\u2019arr\u00eater quand m\u00eame ?',
      detail: 'Occupant(s) : ' + desc,
      okLabel: 'Arr\u00eater quand m\u00eame',
      danger: true,
    });
    if (force) {
      const jf = await postAction({action: 'stop_server_force', id});
      if (jf && jf.ok) toast('Processus tiers arr\u00eat\u00e9', 'ok');
    }
  }
  await load();
}
function copyMcpIdentity(s) {
  const si = (state && state.server_identity) || null;
  const lines = [];
  if (s && s.paired && s.auth_public_key) {
    const kid = s.auth_keyid || 'mcc-server';
    lines.push('# Cl\u00e9 client appair\u00e9e pour ' + (s.label || s.id));
    lines.push('LITEMCP_TRUSTED_KEYS=' + kid + '=' + s.auth_public_key);
    lines.push('MCC_AUTH_KEYID=' + kid);
    lines.push('MCC_AUTH_PUBKEY=' + s.auth_public_key);
    if (s.auth_fingerprint) lines.push('# Empreinte : ' + s.auth_fingerprint);
  } else if (si && si.public_key) {
    const kid = si.keyid || 'mcc-server';
    lines.push('# Identit\u00e9 Ed25519 MCP Control');
    lines.push('LITEMCP_TRUSTED_KEYS=' + kid + '=' + si.public_key);
    lines.push('MCC_AUTH_KEYID=' + kid);
    lines.push('MCC_AUTH_PUBKEY=' + si.public_key);
    if (si.fingerprint) lines.push('# Empreinte : ' + si.fingerprint);
  }
  if (!lines.length) {
    toast('Identit\u00e9 Ed25519 indisponible', 'err');
    return;
  }
  copyText(lines.join('\n'));
}

async function serverPair(id) {
  const j = await postAction({action: 'server_pair', id});
  if (j && j.ok) {
    const kid = j.keyid || 'mcc-server';
    if (j.public_key) {
      copyText('LITEMCP_TRUSTED_KEYS=' + kid + '=' + j.public_key);
      toast('Appair\u00e9 (' + kid + ') \u2014 LITEMCP_TRUSTED_KEYS copi\u00e9e', 'ok');
    } else {
      toast('Serveur appair\u00e9 (' + kid + ')', 'ok');
    }
    await postAction({action: 'refresh', id}, {quiet: true});
  }
  await load();
  if (selectedId === id) {
    const r2 = await fetch('/api/detail?id=' + encodeURIComponent(id));
    if (r2.ok) { detailCache = await r2.json(); renderDetail(); }
  }
}

async function serverUnpair(id) {
  const ok = await askConfirm({
    title: 'Retirer l\u2019auth MCP Control',
    msg: 'Supprimer la cl\u00e9 Ed25519 appair\u00e9e pour ce serveur ? Les requ\u00eates ne seront plus sign\u00e9es RFC 9421.',
    okLabel: 'Retirer l\u2019auth',
    danger: true,
  });
  if (!ok) return;
  const j = await postAction({action: 'server_unpair', id});
  if (j && j.ok) {
    toast('Auth MCP Control retir\u00e9e', 'ok');
    await postAction({action: 'refresh', id}, {quiet: true});
  }
  await load();
  if (selectedId === id) {
    const r2 = await fetch('/api/detail?id=' + encodeURIComponent(id));
    if (r2.ok) { detailCache = await r2.json(); renderDetail(); }
  }
}

async function serverRepair(id) {
  const ok = await askConfirm({
    title: 'R\u00e9-appairer l\u2019auth MCP Control',
    msg: 'G\u00e9n\u00e9rer une nouvelle paire de cl\u00e9s Ed25519 pour ce serveur ? L\u2019ancienne cl\u00e9 sera remplac\u00e9e.',
    okLabel: 'R\u00e9-appairer',
  });
  if (!ok) return;
  const ju = await postAction({action: 'server_unpair', id});
  if (!ju || !ju.ok) return;
  await serverPair(id);
}

async function toggleServerAuthEnv(id, enabled) {
  const j = await postAction({action: 'server_auth_env', id, enabled: !!enabled});
  if (j && j.ok) {
    toast(
      enabled
        ? 'Variables d\u2019auth MCP Control inject\u00e9es (red\u00e9marrez le serveur pour appliquer)'
        : 'Variables d\u2019auth MCP Control retir\u00e9es',
      'ok'
    );
  }
  await load();
  if (selectedId === id) {
    const r2 = await fetch('/api/detail?id=' + encodeURIComponent(id));
    if (r2.ok) { detailCache = await r2.json(); renderDetail(); }
  }
}

// Autoboot par serveur (⚡) : relance auto au démarrage de MCP Control
// et après un crash. Persisté dans config.json par le moteur.
async function setAuto(id, enabled) {
  await postAction({action: 'set_auto_restart', id, enabled}); // refus toasté par le helper (audit 04 #7)
  await load();
  if (selectedId === id) { detailCache = null; await load(); }
}

// Favori (⭐) : épingle la ligne en haut de sa section (persisté). L'épingle
// s'applique À L'AFFICHAGE — l'ordre posé au glisser-déposer reste en base.
async function setFavorite(id, enabled) {
  await postAction({action: 'set_favorite', id, enabled}); // refus toasté par le helper (audit 04 #7)
  await load();
  if (selectedId === id) { detailCache = null; await load(); }
}

// Masquer de l'affichage (session seulement : la supervision moteur continue,
// mais la ligne ne s'affiche plus ni dans la liste ni dans les compteurs).
// Rien n'est persisté : la réouverture de l'app les fait revenir.
function hideServer(id) {
  hiddenServers.add(id);
  if (selectedId === id) { selectedId = null; detailCache = null; renderOverview(); }
  renderServers();
  toast(hiddenServers.size + ' serveur' + (hiddenServers.size > 1 ? 's' : '')
    + ' masqu\u00e9' + (hiddenServers.size > 1 ? 's' : '')
    + ' — il revient \u00e0 la r\u00e9ouverture de l\u2019app');
}
function showHiddenServers() {
  hiddenServers.clear();
  renderServers();
}

/* Scan en un clic : la plage vient des réglages (settings), aucune saisie demandée.
   Retour visuel (audit 04 #10) : bouton occupé (désactivé) pendant le scan,
   puis toast — nombre trouvé (champ `found` de la réponse, consommé s'il
   existe ; le lot 3 l'ajoutera), sinon message neutre. */
let scanBusy = false;
async function doScan() {
  if (scanBusy) return;
  scanBusy = true;
  const btns = [...document.querySelectorAll('button[data-act="scan"]')];
  btns.forEach(b => { b.disabled = true; });
  try {
    const j = await postAction({action: 'scan'}, {quiet: true}); // refus toasté plus bas avec son préfixe
    if (!j.ok) toast('scan : ' + (j.error || 'échec'), 'err');
    else if (Number.isFinite(j.found)) {
      toast(j.found + ' serveur' + (j.found > 1 ? 's' : '') + ' MCP trouvé' + (j.found > 1 ? 's' : ''),
        j.found > 0 ? 'ok' : undefined);
    } else toast('scan terminé — le journal détaille le résultat');
  } finally {
    scanBusy = false;
    btns.forEach(b => { b.disabled = false; });
  }
  await load();
}

async function doScanNet() {
  await postAction({action: 'scan_network'});
  await load();
}

async function refreshAll() {
  // Re-sonde les serveurs connus ET découvre les nouveaux (tous ports en
  // écoute — action discover) en parallèle, un seul rechargement d'état à la
  // fin. Sans toast par serveur (bruit) : un échec GLOBAL est signalé
  // (lot C #1) ; une panne réseau arme de toute façon le bandeau via postAction.
  const ids = (state && state.servers ? state.servers : []).filter(s => s.enabled).map(s => s.id);
  const jobs = ids.map(id => postAction({action: 'refresh', id}, {quiet: true}));
  jobs.push(postAction({action: 'discover'}, {quiet: true}));
  const res = await Promise.all(jobs);
  if (res.some(j => !j.ok)) toast('rafraîchissement global : au moins un appel a échoué', 'err');
  await load();
}

async function doPeers() {
  await postAction({action: 'discover_peers'});
  await load();
}

async function importPeer(ip) {
  await postAction({action: 'import_peer', ip});
  await load();
}

/* ===== Machines distantes (SSH) ===== */
function openMachineDlg() {
  document.getElementById('machineDlg').showModal();
  renderMachines();
}

/* ===== Réglages du client (dialog ⚙ à volets) ===== */
/* Dirty state : tout input/change dans le dialog le marque ; fermer (Esc,
   ×, Annuler) avec des saisies pendantes demande confirmation au lieu de les
   jeter en silence. Le thème, lui, s'applique et se sauve au clic. */
let settingsDirty = false;
/* Instantané des valeurs notifications à l'ouverture du dialog : un
   Enregistrer ne POSTe set_notify que si la section a réellement bougé. */
let notifyOriginal = '';
function setDirty(v) {
  settingsDirty = v;
  const b = document.getElementById('settingsDirty');
  if (b) b.hidden = !v;
}
async function closeSettings() {
  if (settingsDirty) {
    const ok = await askConfirm({
      title: 'Modifications non enregistr\u00e9es',
      msg: 'Modifications non enregistr\u00e9es \u2014 fermer quand m\u00eame ?',
      okLabel: 'Abandonner',
      danger: true,
    });
    if (!ok) return;
  }
  setDirty(false);
  document.getElementById('settingsDlg').close();
  restoreFocus();
}
function setPane(name) {
  document.querySelectorAll('#settingsDlg .set-nav-btn').forEach(b => {
    const sel = b.dataset.pane === name;
    b.classList.toggle('sel', sel);
    b.setAttribute('aria-selected', sel ? 'true' : 'false');
    b.setAttribute('tabindex', sel ? '0' : '-1');
  });
  document.querySelectorAll('#settingsDlg .set-pane').forEach(p => p.classList.toggle('sel', p.dataset.pane === name));
  try { localStorage.setItem('mcc.setpane', name); } catch (e) {}
}
/* Labels humains des clés renvoyées par set_settings : le message parle
   utilisateur (« plage de scan »), pas clés internes (« scan »). */
const SETTINGS_LABELS = {
  refresh: 'intervalle de sondage',
  scan: 'plage de scan',
  ui_bind: '\u00e9coute',
  network: 'r\u00e9seau',
  network_ports: 'ports UI des pairs',
  discovery_ports_only: 'liste blanche de d\u00e9couverte',
  discovery_ports_excluded: 'ports exclus de la d\u00e9couverte',
};
function setFieldErr(id, msg) {
  const el = document.getElementById('err_' + id);
  if (!el) return;
  if (msg) { el.textContent = msg; el.hidden = false; }
  else { el.textContent = ''; el.hidden = true; }
}
function clearFieldErrs() {
  ['refresh', 'scan', 'ports', 'disc_only', 'disc_excl', 'webhook', 'states', 'min'].forEach(id => setFieldErr(id, ''));
}
function showMsg(rowId, msgId, text) {
  const row = document.getElementById(rowId);
  const el = document.getElementById(msgId);
  if (row) row.hidden = false;
  if (el) el.textContent = text;
}

function openSettingsDlg() {
  const s = state.settings || {};
  const n = s.notify || {};
  document.getElementById('sRefresh').value = s.refresh_secs || 15;
  document.getElementById('sScanFrom').value = s.scan_from || 3000;
  document.getElementById('sScanTo').value = s.scan_to || 3200;
  document.getElementById('sNetEnabled').checked = !!s.network_enabled;
  document.getElementById('sNetPorts').value = (s.network_ports || []).join(', ');
  const discOnlyEl = document.getElementById('sDiscOnly');
  if (discOnlyEl) discOnlyEl.value = (s.discovery_ports_only || []).join(', ');
  const discExclEl = document.getElementById('sDiscExcl');
  if (discExclEl) discExclEl.value = (s.discovery_ports_excluded || []).join(', ');
  document.getElementById('sUiBind').value = s.ui_bind || '127.0.0.1';
  document.getElementById('sUiPort').value = s.ui_port || 5090;
  // Notifications (moteur : action set_notify, section notify de config.json).
  // Lot 5 : les en-têtes ne sortent plus de l'API (headers_set) — le champ vide
  // signifie « inchangés » : le moteur ne reçoit la clé `headers` que si
  // l'utilisateur en saisit de nouvelles ; https:// est refusé par le moteur
  // (aucune pile TLS).
  document.getElementById('sNotifDesktop').checked = !!n.desktop;
  const urlField = document.getElementById('sWebhookUrl');
  urlField.value = n.webhook_url || '';
  urlField.placeholder = 'http://hooks.exemple.com/services/\u2026 (https non pris en charge)';
  document.getElementById('sWebhookMethod').value = n.method === 'PUT' ? 'PUT' : 'POST';
  const hdrField = document.getElementById('sWebhookHeaders');
  hdrField.value = '';
  hdrField.placeholder = n.headers_set
    ? 'En-t\u00eates d\u00e9j\u00e0 configur\u00e9s (masqu\u00e9s) \u2014 laisser vide = inchang\u00e9s'
    : 'Authorization: Bearer \u2026 (une paire \u00ab Cl\u00e9: valeur \u00bb par ligne)';
  const states = n.states || ['DOWN', 'PORT-ONLY'];
  const hasState = v => states.some(x => String(x).toUpperCase() === v);
  document.getElementById('sStDown').checked = hasState('DOWN');
  document.getElementById('sStPortOnly').checked = hasState('PORT-ONLY');
  document.getElementById('sNotifMin').value = (n.min_interval_secs ?? 300);
  notifyOriginal = JSON.stringify(notifyFormValues());
  // sVersion/sLatest sont des <span> (affichage), pas des champs éditables
  document.getElementById('sVersion').textContent = 'v' + MCC_VERSION;
  document.getElementById('sLatest').textContent = state.latest_version ? 'v' + state.latest_version : '\u2014';
  const upd = document.getElementById('updMsg');
  upd.textContent = state.update_available ? '\u2b06 v' + state.latest_version + ' disponible' : 'Aucune mise \u00e0 jour connue (auto-v\u00e9rification au boot puis chaque heure).';
  upd.classList.remove('tone-warn', 'tone-ok');
  if (state.update_available) upd.classList.add('tone-warn');
  const row = document.getElementById('resetMsgRow');
  if (row) row.hidden = true;
  document.getElementById('resetMsg').textContent = '';
  ['notifyMsgRow', 'runtimeMsgRow'].forEach(id => { const el = document.getElementById(id); if (el) el.hidden = true; });
  ['notifyMsg', 'runtimeMsg'].forEach(id => { const el = document.getElementById(id); if (el) el.textContent = ''; });
  document.getElementById('settingsMsg').textContent = (s.tls && s.tls.notice) ? s.tls.notice : '';
  clearFieldErrs();
  setDirty(false);
  // dernier volet consulté (préférence locale)
  let pane = 'general';
  try {
    const p = localStorage.getItem('mcc.setpane');
    if (p && document.querySelector('#settingsDlg .set-pane[data-pane="' + p + '"]')) pane = p;
  } catch (e) {}
  setPane(pane);
  const foot = document.getElementById('settingsFootNote');
  if (foot) foot.textContent = 'Les r\u00e9glages sont enregistr\u00e9s dans config.json.';
  applyTheme(document.documentElement.dataset.theme === 'light' ? 'light' : 'dark');
  rememberFocus();
  document.getElementById('settingsDlg').showModal();
  const nav = document.querySelector('#settingsDlg .set-nav-btn.sel');
  if (nav) nav.focus();
}
async function checkUpdateNow() {
  const msg = document.getElementById('updMsg');
  msg.classList.remove('tone-warn', 'tone-ok');
  msg.textContent = '⏳ vérification de la release GitHub…';
  const j = await postAction({action: 'check_update'}, {quiet: true});
  const shaRow = document.getElementById('sSha256Row');
  const shaAsset = document.getElementById('sSha256Asset');
  const shaVal = document.getElementById('sSha256Val');
  const shaCopy = document.getElementById('sSha256Copy');
  const shaNote = document.getElementById('sSha256Note');
  if (shaRow) {
    if (j.sha256 && typeof j.sha256 === 'object') {
      shaRow.hidden = false;
      if (shaAsset) shaAsset.textContent = j.sha256.asset || 'binaire';
      if (j.sha256.sha256) {
        if (shaVal) shaVal.textContent = j.sha256.sha256;
        if (shaCopy) shaCopy.hidden = false;
        if (shaNote) shaNote.textContent = 'Empreinte SHA-256 publi\u00e9e dans SHA256SUMS.txt de la release GitHub.';
      } else {
        if (shaVal) shaVal.textContent = j.sha256.note || 'non publi\u00e9e';
        if (shaCopy) shaCopy.hidden = true;
        if (shaNote) shaNote.textContent = j.sha256.note || 'Empreinte non disponible pour cet actif.';
      }
    } else {
      shaRow.hidden = true;
    }
  }
  if (j.ok && j.checked && j.update_available) {
    document.getElementById('sLatest').textContent = 'v' + j.latest;
    msg.textContent = '\u2b06 v' + j.latest + ' disponible (installée : v' + j.current + ') — bouton « Mettre à jour » en bas à droite';
    msg.classList.add('tone-warn');
  } else if (j.ok && j.checked) {
    document.getElementById('sLatest').textContent = j.latest ? 'v' + j.latest : '—';
    msg.textContent = '✓ à jour — vérifié à l\u2019instant : v' + j.current + ' est la dernière release';
    msg.classList.add('tone-ok');
  } else {
    msg.textContent = j.latest
      ? '✗ vérification impossible (réseau ?) — dernière info connue : v' + j.latest + ' disponible'
      : '✗ vérification impossible (réseau ?) — nouvel essai auto au boot puis chaque heure';
  }
}
function copySha256() {
  const el = document.getElementById('sSha256Val');
  if (el && el.textContent && el.textContent !== '\u2013') copyText(el.textContent);
}
async function openReleasePage() {
  // window.open est neutralisé dans la webview Tauri (desktop) : le clic mort
  // « rien ne se passe ». On passe par le moteur, qui lance le navigateur par
  // défaut de la machine (action open_release_page, URL fixe sans argument).
  // Repli : si le moteur ne peut pas (headless sans navigateur), on tente
  // window.open — utile quand l'UI tourne dans un vrai navigateur.
  const j = await postAction({action: 'open_release_page'}, {quiet: true});
  if (j.ok) return;
  // moteur injoignable ou refus : repli window.open (navigateur classique)
  window.open('https://github.com/DeamonDev888/mcp-control/releases/latest', '_blank', 'noopener,noreferrer');
}
/* OS du navigateur — la machine où atterrit le téléchargement quand l'UI est
   servie par un moteur distant (VPS). Le moteur valide et retombe sur le sien. */
function browserOs() {
  const ua = navigator.userAgent || '';
  if (ua.includes('Windows')) return 'windows';
  if (ua.includes('Mac OS') || ua.includes('Macintosh')) return 'macos';
  if (ua.includes('Linux') && !ua.includes('Android')) return 'linux';
  return null;
}
let lastUpdateUrl = null;
let _updatingNow = false;
/* ⬆ Mettre à jour & relancer : téléchargement HTTPS + vérification SHA-256 +
   installation silencieuse (NSIS /S) ou remplacement binaire + redémarrage
   automatique de la nouvelle version. Repli manuel (`download_update`) si
   l'auto-installation échoue. */
async function downloadUpdate() {
  if (_updatingNow) return;
  const ver = (state && state.latest_version) ? ('v' + state.latest_version) : 'la derni\u00e8re version';
  const ok = await askConfirm({
    title: 'Mettre \u00e0 jour et relancer',
    msg: 'T\u00e9l\u00e9charger ' + ver + ', v\u00e9rifier son empreinte SHA-256, installer automatiquement et relancer MCP Control ?',
    okLabel: 'Installer et relancer',
    cancelLabel: 'Annuler',
  });
  if (!ok) return;
  _updatingNow = true;
  const btn = document.getElementById('btnUpdInstall');
  const msg = document.getElementById('updMsg');
  if (btn) btn.disabled = true;
  if (msg) {
    msg.classList.remove('tone-warn', 'tone-ok');
    msg.textContent = '\u23f3 T\u00e9l\u00e9chargement et v\u00e9rification SHA-256 de ' + ver + '\u2026';
  }
  toast('T\u00e9l\u00e9chargement et v\u00e9rification SHA-256 en cours\u2026', 'ok');
  const j = await postAction({action: 'install_update'}, {quiet: true});
  if (j.ok && j.restarting) {
    if (msg) {
      msg.classList.add('tone-ok');
      msg.textContent = '\u2713 v' + (j.version || '') + ' v\u00e9rifi\u00e9e (SHA-256 OK) \u2014 installation et red\u00e9marrage automatique\u2026';
    }
    toast('Mise \u00e0 jour v' + (j.version || '') + ' v\u00e9rifi\u00e9e \u2014 red\u00e9marrage en cours\u2026', 'ok');
    setTimeout(() => { _updatingNow = false; if (btn) btn.disabled = false; load(); }, 4000);
    return;
  }
  _updatingNow = false;
  if (btn) btn.disabled = false;
  if (j.netDown) {
    if (lastUpdateUrl) window.open(lastUpdateUrl, '_blank', 'noopener,noreferrer');
    else toast('moteur injoignable \u2014 r\u00e9essaie plus tard', 'err');
    return;
  }
  if (msg) {
    msg.classList.add('tone-warn');
    msg.textContent = '\u2717 ' + (j.error || '\u00e9chec de la mise \u00e0 jour automatique');
  }
  // Si l'auto-mise à jour a échoué (ex. déjà à jour ou refus), afficher l'erreur
  // et proposer le téléchargement manuel si ce n'est pas « déjà à jour ».
  toast(j.error || '\u00e9chec de la mise \u00e0 jour', 'err');
  if (j.error && !String(j.error).includes('d\u00e9j\u00e0 \u00e0 jour') && !String(j.error).includes('SHA-256')) {
    const manual = {action: 'download_update'};
    const os = browserOs();
    if (os) manual.os = os;
    const dl = await postAction(manual, {quiet: true});
    if (dl.url) lastUpdateUrl = dl.url;
  }
}
async function resetState(what) {
  const sel = document.getElementById('sResetWhat');
  const label = sel.options[sel.selectedIndex].text;
  const ok = await askConfirm({
    title: 'R\u00e9initialiser les donn\u00e9es',
    msg: 'R\u00e9initialiser \u00ab ' + label + ' \u00bb ? Action irr\u00e9versible.',
    okLabel: 'R\u00e9initialiser',
    danger: true,
  });
  if (!ok) return;
  const msg = document.getElementById('resetMsg');
  const row = document.getElementById('resetMsgRow');
  if (row) row.hidden = false;
  msg.textContent = '⏳ réinitialisation…';
  const j = await postAction({action: 'reset_state', what}, {quiet: true});
  if (!j.ok) msg.textContent = '✗ ' + (j.error || 'échec');
  else {
    const det = (j.reinjected ? ' · ' + j.reinjected + ' seed(s) réinjecté(s)' : '') + (j.removed ? ' · ' + j.removed + ' serveur(s) retiré(s)' : '');
    msg.textContent = '✓ Réinitialisé (' + (j.cleared || []).join(', ') + ')' + det;
    await load();
  }
}
/* Valeurs du formulaire notifications, telles quelles (sans le champ action) :
   sérialisées pour détecter si la section a réellement changé (évite un
   « configuration enregistrée » dans le journal à chaque Enregistrer). */
function notifyFormValues() {
  return {
    desktop: document.getElementById('sNotifDesktop').checked,
    webhook_url: document.getElementById('sWebhookUrl').value.trim(),
    method: document.getElementById('sWebhookMethod').value,
    // Lot 5 : champ vide = en-têtes inchangés — la clé n'est PAS envoyée et
    // le moteur conserve ceux déjà configurés (masqués côté API).
    headers: document.getElementById('sWebhookHeaders').value.trim() === ''
      ? undefined
      : document.getElementById('sWebhookHeaders').value,
    states: [
      ...(document.getElementById('sStDown').checked ? ['DOWN'] : []),
      ...(document.getElementById('sStPortOnly').checked ? ['PORT-ONLY'] : []),
    ],
    min_interval_secs: +document.getElementById('sNotifMin').value || 0,
  };
}
/* Lecture + validation de TOUS les champs du dialog : chaque erreur est
   affichée sous sa ligne (set-err), jamais une seule ligne globalisée.
   Renvoie null si invalide, sinon le corps d'action set_settings et, si la
   section Notifications a changé, le corps d'action set_notify. */
function collectSettingsForm() {
  let ok = true;
  const intOrNull = id => { const raw = document.getElementById(id).value.trim(); return raw === '' ? null : +raw; };
  // Intervalle de sondage : entier 5..3600
  const refresh = intOrNull('sRefresh');
  if (refresh === null || !Number.isInteger(refresh) || refresh < 5 || refresh > 3600) {
    setFieldErr('refresh', 'entier entre 5 et 3600 secondes'); ok = false;
  } else setFieldErr('refresh', '');
  // Plage de scan : entiers 1..65535, bornes échangées si inversées
  let sf = intOrNull('sScanFrom'), st = intOrNull('sScanTo');
  let swapped = false;
  if (sf === null || st === null || !Number.isInteger(sf) || !Number.isInteger(st)
      || sf < 1 || sf > 65535 || st < 1 || st > 65535) {
    setFieldErr('scan', 'ports entiers entre 1 et 65535'); ok = false;
    sf = null; st = null;
  } else {
    if (sf > st) { const t = sf; sf = st; st = t; swapped = true; }
    setFieldErr('scan', '');
  }
  // Ports UI des pairs : liste d'entiers 1..65535
  const rawPorts = document.getElementById('sNetPorts').value.trim();
  const tokens = rawPorts ? rawPorts.split(/[\s,]+/).filter(Boolean) : [];
  const bad = tokens.filter(t => !/^\d+$/.test(t) || +t < 1 || +t > 65535);
  if (bad.length) { setFieldErr('ports', 'ports invalides : ' + bad.slice(0, 3).join(', ')); ok = false; }
  else setFieldErr('ports', '');
  const netPorts = tokens.filter(t => /^\d+$/.test(t)).map(Number).filter(p => p >= 1 && p <= 65535);
  // Découverte locale : liste blanche (discovery_ports_only) et exclusions (discovery_ports_excluded)
  const parseDiscField = (inputId, errId) => {
    const el = document.getElementById(inputId);
    const raw = el ? el.value.trim() : '';
    const toks = raw ? raw.split(/[\s,]+/).filter(Boolean) : [];
    const badToks = toks.filter(t => !/^\d+$/.test(t) || +t < 1 || +t > 65535);
    if (badToks.length) { setFieldErr(errId, 'ports invalides : ' + badToks.slice(0, 3).join(', ')); ok = false; return []; }
    setFieldErr(errId, '');
    return [...new Set(toks.map(Number).filter(p => p >= 1 && p <= 65535))];
  };
  const discOnly = parseDiscField('sDiscOnly', 'disc_only');
  const discExcl = parseDiscField('sDiscExcl', 'disc_excl');
  // Webhook : vide autorisé (canal off), sinon URL http:// uniquement —
  // https:// est refusé par le moteur à l'enregistrement (aucune pile TLS :
  // l'envoi partait en HTTP brut sur le port 443, audit 05 #4).
  const url = document.getElementById('sWebhookUrl').value.trim();
  if (url && !/^http:\/\//i.test(url)) { setFieldErr('webhook', 'URL http:// attendue (https non pris en charge : aucune pile TLS)'); ok = false; }
  else setFieldErr('webhook', '');
  // États notifiés : au moins un (sinon aucun incident ne déclencherait rien)
  const notify = notifyFormValues();
  notify.webhook_url = url;
  if (!notify.states.length) { setFieldErr('states', 'coche au moins un état (DOWN ou PORT-ONLY)'); ok = false; }
  else setFieldErr('states', '');
  // Délai anti-spam : borné 0..86400 (le moteur clampe déjà, on affiche juste)
  notify.min_interval_secs = Math.min(86400, Math.max(0, notify.min_interval_secs));
  if (!ok) return null;
  return {
    swapped,
    settings: {
      action: 'set_settings',
      refresh_secs: refresh,
      scan_from: sf,
      scan_to: st,
      ui_bind: document.getElementById('sUiBind').value,
      network_enabled: document.getElementById('sNetEnabled').checked,
      network_ports: netPorts,
      discovery_ports_only: discOnly,
      discovery_ports_excluded: discExcl,
    },
    notifyChanged: JSON.stringify(notify) !== notifyOriginal,
    notify,
  };
}
async function saveSettings() {
  const form = collectSettingsForm();
  if (!form) return false;
  const msg = document.getElementById('settingsMsg');
  const parts = [];
  const j = await postAction(form.settings, {quiet: true});
  if (!j.ok) { msg.textContent = '\u2717 ' + (j.error || '\u00e9chec'); return false; }
  parts.push((j.changed || []).length
    ? '\u2713 enregistr\u00e9 : ' + j.changed.map(k => SETTINGS_LABELS[k] || k).join(', ')
    : 'r\u00e9glages inchang\u00e9s');
  if ((j.changed || []).includes('ui_bind')) parts.push('l\u2019\u00e9coute s\u2019applique au prochain d\u00e9marrage');
  if (form.swapped) parts.push('plage de scan r\u00e9ordonn\u00e9e (du plus petit au plus grand port)');
  if (form.notifyChanged) {
    const j2 = await postAction(Object.assign({action: 'set_notify'}, form.notify), {quiet: true});
    parts.push(j2.ok ? 'notifications enregistr\u00e9es' : '\u2717 notifications : ' + (j2.error || '\u00e9chec'));
  }
  msg.textContent = parts.join(' \u00b7 ');
  setDirty(false);
  await load();
  armPolling();
  return true;
}
/* Test des notifications : enregistre d'abord les saisies pendantes (le
   moteur teste la config PERSISTÉE), puis émet sur chaque canal activé. */
async function testNotify() {
  if (settingsDirty) { const saved = await saveSettings(); if (!saved) return; }
  showMsg('notifyMsgRow', 'notifyMsg', '\u23f3 test en cours\u2026');
  const j = await postAction({action: 'test_notify'}, {quiet: true});
  if (!j.ok) { showMsg('notifyMsgRow', 'notifyMsg', '\u2717 ' + (j.error || '\u00e9chec')); return; }
  const parts = [];
  if (j.desktop) parts.push(j.desktop.ok ? 'desktop \u2713 (toast affich\u00e9)' : 'desktop \u2717 \u2014 ' + (j.desktop.error || '?'));
  if (j.webhook) parts.push(j.webhook.ok ? 'webhook \u2713' : 'webhook \u2717 \u2014 ' + (j.webhook.error || '?'));
  showMsg('notifyMsgRow', 'notifyMsg', parts.join(' \u00b7 ') || 'aucun canal actif');
}
/* Historique runtime : crashs, relances, backoff — reset_runtime du moteur. */
async function resetRuntime() {
  const ok = await askConfirm({
    title: 'Effacer l\u2019historique runtime',
    msg: 'Effacer l\u2019historique runtime (crashs, relances, backoff) ?',
    okLabel: 'Effacer',
    danger: true,
  });
  if (!ok) return;
  const j = await postAction({action: 'reset_runtime'}, {quiet: true});
  showMsg('runtimeMsgRow', 'runtimeMsg',
    j.ok ? '\u2713 historique effac\u00e9 (' + (j.cleared || 0) + ' serveur(s))' : '\u2717 ' + (j.error || '\u00e9chec'));
  await load();
}
/* Câblage du dialog (une fois au boot) : dirty state sur toute saisie, et
   Esc = event cancel. */
(function () {
  const d = document.getElementById('settingsDlg');
  if (!d) return;
  d.addEventListener('input', () => setDirty(true));
  d.addEventListener('change', () => setDirty(true));
  d.addEventListener('cancel', (e) => {
    if (settingsDirty) {
      e.preventDefault();
      closeSettings();
    } else {
      setDirty(false);
      restoreFocus();
    }
  });
})();
function renderMachines() {
  if (!state) return;
  const hosts = state.settings.ssh_hosts || [];
  const peers = state.peers || [];
  const box = document.getElementById('machineList');
  let html = hosts.length ? '' :
    '<div class="empty-sec">aucune machine enregistr&eacute;e — entre une IP et un utilisateur SSH ci-dessus</div>';
  html += hosts.map(h => {
    const peer = peers.find(p => p.ip === h.ip);
    const st = peer ? `moteur MCP Control d&eacute;tect&eacute; — ${esc((peer.stats||{}).up ?? '?')} UP / ${esc((peer.stats||{}).total ?? '?')} serveurs`
                    : 'machine enregistr&eacute;e';
    return `<div class="machine-row">
      <div class="l1"><span class="dot UP"></span><span class="nm">${esc(h.ip)}</span><span class="tag http">${esc(h.user)}</span>${peer ? '<span class="tag sse">pair</span>' : ''}</div>
      <div class="l2">${st}</div>
      <div class="macts">
        <button class="primary" data-act="ssh-discover" data-id="${esc(h.ip)}" data-user="${esc(h.user)}">${ic('link')} SSH &amp; d&eacute;couvrir</button>
        ${peer ? `<button data-act="import-peer" data-id="${esc(h.ip)}">Importer ses serveurs</button>` : ''}
        <button class="danger" data-act="ssh-forget" data-id="${esc(h.ip)}">Oublier</button>
      </div>
    </div>`;
  }).join('');
  const autres = peers.filter(p => !hosts.find(h => h.ip === p.ip));
  html += autres.map(p => `<div class="machine-row">
      <div class="l1"><span class="dot UP"></span><span class="nm">${esc(p.ip)}</span><span class="tag sse">pair</span></div>
      <div class="l2">UI :${esc(p.ui_port)} &mdash; ${esc((p.stats||{}).up ?? '?')} UP / ${esc((p.stats||{}).total ?? '?')} serveurs</div>
      <div class="macts"><button data-act="import-peer" data-id="${esc(p.ip)}">Importer ses serveurs</button></div>
    </div>`).join('');
  box.innerHTML = html;
}
function sshForm() {
  const ip = (document.getElementById('mIp').value || '').trim();
  const user = (document.getElementById('mUser').value || '').trim();
  return {ip, user};
}
async function sshDiscover(ip, user) {
  const f = sshForm();
  ip = ip || f.ip; user = user || f.user;
  if (!ip || !user) { toast('IP et utilisateur SSH requis', 'err', 3500); return; }
  const msg = document.getElementById('machineMsg');
  msg.textContent = '⏳ connexion SSH + netstat distant + sonde des ports… (jusqu\u2019à ~30 s, vois le journal)';
  const j = await postAction({action: 'ssh_discover', ip, user}, {quiet: true});
  if (j.ok && j.started) {
    msg.textContent = '⏳ d\u00e9couverte lanc\u00e9e sur ' + ip + ' — le journal et la liste se remplissent pendant la sonde.';
    let t = 0;
    const iv = setInterval(async () => {
      await load(); renderMachines();
      if (++t >= 10) { clearInterval(iv); }
    }, 4000);
  } else if (!j.ok) {
    msg.textContent = '✗ ' + (j.error || 'échec');
  }
}
async function sshSave() {
  const f = sshForm();
  if (!f.ip || !f.user) { toast('IP et utilisateur SSH requis', 'err', 3500); return; }
  const j = await postAction({action: 'ssh_save', ip: f.ip, user: f.user}, {quiet: true});
  document.getElementById('machineMsg').textContent = j.ok ? '✓ machine enregistr\u00e9e' : '✗ ' + (j.error || 'erreur');
  await load(); renderMachines();
}
async function sshForget(ip) {
  await postAction({action: 'ssh_forget', ip});
  await load(); renderMachines();
}

async function toggleTool(id, name) {
  await postAction({action: 'toggle_tool', id, tool: name});
  const r = await fetch('/api/detail?id=' + encodeURIComponent(id));
  if (r.ok) { detailCache = await r.json(); renderDetail(); }
  await load();
}

function toggleAdd() { rememberFocus(); document.getElementById('dlg').showModal(); }
function transportChanged() {
  const stdio = document.getElementById('fTransport').value === 'stdio';
  document.getElementById('httpFields').hidden = stdio;
  document.getElementById('stdioFields').hidden = !stdio;
}

/* Import d'un bloc mcpServers / URL / commande (audit 04 #4) : le moteur
   parse tout (import_mcp) ; l'UI affiche le nombre ajouté et, si la réponse
   les porte, les entrées écartées avec leur motif (champ `skipped`). */
async function importMcp() {
  const ta = document.getElementById('fImport');
  const msg = document.getElementById('importMsg');
  const text = (ta.value || '').trim();
  if (!text) { msg.textContent = '\u2717 colle un bloc mcpServers, une URL http:// ou une commande (npx, claude mcp add…)'; return; }
  msg.textContent = '\u23f3 import\u2026';
  const j = await postAction({action: 'import_mcp', text}, {quiet: true});
  if (!j.ok) { msg.textContent = '\u2717 ' + (j.error || 'échec'); return; }
  const parts = ['\u2713 ' + (j.added || 0) + ' serveur' + ((j.added || 0) > 1 ? 's' : '') + ' ajouté' + ((j.added || 0) > 1 ? 's' : '')];
  const skipped = Array.isArray(j.skipped) ? j.skipped : [];
  if (skipped.length) {
    parts.push('écartés : ' + skipped.map(it => typeof it === 'string' ? it
      : ((it.name || it.id || 'entrée') + ((it.reason || it.error || it.motif) ? ' — ' + (it.reason || it.error || it.motif) : ''))).join(' ; '));
  }
  if (j.note) parts.push(j.note);
  await load();
  // succès complet : on referme (le toast rappelle le total) ; entrées
  // écartées ou rien d'ajouté : le dialog reste ouvert pour lire les motifs.
  if ((j.added || 0) > 0 && !skipped.length) {
    ta.value = '';
    msg.textContent = '';
    document.getElementById('dlg').close();
    toast(parts.join(' — '), 'ok');
  } else {
    msg.textContent = parts.join(' — ');
  }
}

async function addServer(ev) {
  if (ev && ev.preventDefault) ev.preventDefault();
  const t = document.getElementById('fTransport').value;
  const body = {action: 'add', transport: t, label: document.getElementById('fLabel').value};
  if (t === 'http') {
    body.port = +document.getElementById('fPort').value;
    body.path = document.getElementById('fPath').value || '/mcp';
    const prog = document.getElementById('fHttpProgram').value.trim();
    if (prog) {
      body.program = prog;
      const wd = document.getElementById('fHttpWorkdir').value.trim();
      if (wd) body.workdir = wd;
      body.args = document.getElementById('fHttpArgs').value.split(/\s+/).filter(Boolean);
    }
  }
  else { body.program = document.getElementById('fProgram').value;
         body.workdir = document.getElementById('fWorkdir').value || null;
         body.args = document.getElementById('fArgs').value.split(/\s+/).filter(Boolean); }
  const j = await postAction(body, {quiet: true});
  if (!j.ok) toast(j.error || 'erreur', 'err', 4000);
  else { document.getElementById('dlg').close(); restoreFocus(); selectedId = j.id; await act('refresh', j.id); }
  return false;
}

/* ===== Boutique MCP (porté de Hermes mcp-import.ts / connectors-directory) ===== */
let storeItems = null;
let storeLogos = null;
let storeCat = 'Tout';
async function openStore() {
  document.getElementById('storeDlg').showModal();
  if (!storeItems) {
    const r = await fetch('/api/store');
    storeItems = await r.json();
    document.getElementById('storeCount').textContent = '(' + storeItems.length + ' MCP)';
    // logos officiels (glyphes embarqués au binaire, servis en JSON)
    fetch('/api/store-logos').then(r2 => r2.json()).then(j => {
      storeLogos = j;
      renderStoreItems(); // re-rendu quand les logos arrivent
    }).catch(() => {});
    const cats = ['Tout', ...new Set(storeItems.map(i => i.cat))];
    document.getElementById('storeCats').innerHTML = cats.map(c =>
      `<button class="scat ${c===storeCat?'sel':''}" data-act="store-cat" data-cat="${esc(c)}">${esc(c)}</button>`).join('');
  }
  renderStoreItems();
}
/* Logo d'une carte : glyphe officiel par id (STORE_LOGOS), sinon pour les items
   aitmpl une marque reconnue dans la commande d'installation, sinon un glyphe
   « paquet » neutre — TOUTE carte porte une icône (jamais de carte sans logo,
   avant comme après le chargement de /api/store-logos). NB : guillemet-double
   suivi de dièse interdit dans ce bloc — la séquence fermerait la raw string
   Rust qui embarque cette page. */
const PKG_GLYPH = "<path fill='none' stroke='#8a93a5' stroke-width='2' stroke-linejoin='round' d='m12 2 9 5v10l-9 5-9-5V7Zm0 10 9-5m-9 5-9-5m9 5v10'/>";
const PKG_LOGOS = [
  ['context7', 'context7'], ['mongodb', 'mongodb'], ['supabase', 'supabase'],
  ['playwright', 'playwright'], ['figma', 'figma'], ['firecrawl', 'firecrawl'],
  ['notionhq', 'notion'], ['stripe', 'stripe'], ['kubernetes', 'kubernetes'],
  ['duckduckgo', 'duckduckgo'], ['todoist', 'todoist'], ['sqlite', 'sqlite'],
  ['postgres', 'postgresql'], ['server-gdrive', 'googledrive'], ['slack', 'slack'],
  ['brave', 'brave'], ['n8n', 'n8n'], ['gitlab', 'gitlab'], ['sentry', 'sentry'],
  ['server-google-maps', 'googlemaps'], ['paddleocr', 'paddlepaddle'],
  ['chrome-devtools', 'googlechrome'],
];
function storeLogoSvg(inner) {
  return `<span class="slogo-well"><svg class="slogo" viewBox="0 0 24 24" aria-hidden="true">${inner}</svg></span>`;
}
function storeLogo(i) {
  if (!storeLogos) return storeLogoSvg(PKG_GLYPH);
  // glyphe officiel par id, préfixes de catalogues additionnels retirés
  const inner = storeLogos[i.id] || storeLogos[i.id.replace(/^(at|gl)-/, '')];
  if (inner) return storeLogoSvg(inner);
  // items de catalogues additionnels : marque reconnue dans la commande
  if (/^(at|gl|gh)-/.test(i.id)) {
    const cmd = (i.program + ' ' + (i.args || []).join(' ')).toLowerCase();
    for (const [needle, lid] of PKG_LOGOS) {
      if (storeLogos[lid] && cmd.includes(needle)) return storeLogoSvg(storeLogos[lid]);
    }
  }
  // glyphe « paquet » neutre pour tout ce qui n'a pas de marque reconnue
  return storeLogoSvg(PKG_GLYPH);
}
/* Id serveur d'un item boutique : install_store clé sur « store-<id> ». */
const storeSrvId = i => 'store-' + i.id;
let storeHideInst = false;
function toggleHideInst() {
  storeHideInst = !storeHideInst;
  const b = document.getElementById('storeHideInst');
  if (b) {
    b.classList.toggle('sel', storeHideInst);
    b.setAttribute('aria-checked', storeHideInst ? 'true' : 'false');
  }
  renderStoreItems();
}
function storeEnvKeys(i) {
  const desc = String((i && i.desc) || '');
  const matches = desc.match(/\b[A-Z][A-Z0-9_]{3,}\b/g) || [];
  const keys = [...new Set(matches.filter(k => k.includes('_')))];
  if (!keys.length && /tokens?\s+requis\s+en\s+env/i.test(desc)) {
    return ['API_TOKEN'];
  }
  return keys;
}
function toggleEnvEye(btn) {
  const row = btn && btn.closest ? btn.closest('.scard-env-row') : null;
  const inp = row ? row.querySelector('input.store-env-inp') : null;
  if (!inp) return;
  const show = inp.type === 'password';
  inp.type = show ? 'text' : 'password';
  btn.innerHTML = ic(show ? 'eyeOff' : 'eye');
}
function renderStoreItems() {
  if (!storeItems) return;
  const q = (document.getElementById('storeSearch').value || '').toLowerCase();
  const sort = document.getElementById('storeSort').value;
  const installed = new Set((state && state.servers ? state.servers : []).map(s => s.id));
  let list = storeItems.filter(i => (storeCat==='Tout' || i.cat===storeCat) &&
    (!q || i.name.toLowerCase().includes(q) || i.desc.toLowerCase().includes(q)));
  if (storeHideInst) list = list.filter(i => !installed.has(storeSrvId(i)));
  if (sort === 'az') list = [...list].sort((a, b) => a.name.localeCompare(b.name, 'fr', {sensitivity: 'base'}));
  const res = document.getElementById('storeResults');
  res.textContent = list.length === storeItems.length
    ? storeItems.length + ' serveurs'
    : list.length + ' affiché' + (list.length > 1 ? 's' : '') + ' sur ' + storeItems.length;
  document.getElementById('storeGrid').innerHTML = list.map(i => {
    const isInst = installed.has(storeSrvId(i));
    const envKeys = !isInst ? storeEnvKeys(i) : [];
    const envHtml = envKeys.length ? `<div class="scard-env">${envKeys.map(k =>
      `<div class="scard-env-row">
        <input type="password" class="store-env-inp" data-env="${esc(k)}" placeholder="${esc(k)}" aria-label="Variable ${esc(k)}" autocomplete="off">
        <button type="button" class="eye-btn" data-act="toggle-env-eye" aria-label="Afficher ou masquer ${esc(k)}" title="Afficher / masquer">${ic('eye')}</button>
      </div>`).join('')}</div>` : '';
    return `<div class="scard">
      <div class="scard-top">
        ${storeLogo(i)}
        <div class="scard-title">
          <b>${esc(i.name)}</b>
          <span class="scard-cat">${esc(i.cat)}</span>
        </div>
        <span class="tag ${i.kind==='npm'?'npm':'py'}">${esc(i.kind)}</span>
      </div>
      <div class="scard-desc">${esc(i.desc)}</div>
      ${envHtml}
      <div class="scard-foot">
        ${isInst
          ? `<button class="btn-inst" disabled title="Ce serveur est d&eacute;j&agrave; dans ta liste">${ic('check')} D&eacute;j&agrave; install&eacute;</button>`
          : `<button class="primary" data-act="install-store" data-id="${esc(i.id)}">${ic('download')} Installer</button>`}
      </div>
    </div>`;
  }).join('') || '<div class="empty-sec">aucun r&eacute;sultat</div>';
}
async function installStore(id, btn) {
  const card = btn && btn.closest ? btn.closest('.scard') : null;
  const envInps = card ? [...card.querySelectorAll('input.store-env-inp')] : [];
  const envMap = {};
  let hasEnv = false;
  envInps.forEach(inp => {
    const k = inp.dataset.env;
    const v = (inp.value || '').trim();
    if (k && v) { envMap[k] = v; hasEnv = true; }
    inp.value = '';
    inp.type = 'password';
  });
  btn.disabled = true; btn.textContent = '\u23f3 Installation\u2026';
  const body = {action: 'install_store', id};
  if (hasEnv) body.env = envMap;
  if (id === 'filesystem') {
    const res = await askConfirm({
      title: 'Dossier Filesystem',
      msg: 'Indiquez le dossier \u00e0 exposer au serveur Filesystem (chemin complet) :',
      inputs: [{id: 'dir', label: 'Chemin du dossier autoris\u00e9', placeholder: 'C:\\Users\\...\\mon-projet'}],
      okLabel: 'Installer',
    });
    const dir = res && typeof res === 'object' ? res.dir : '';
    if (!dir || !dir.trim()) {
      btn.disabled = false; btn.innerHTML = ic('download') + ' Installer';
      return;
    }
    body.args = ['-y', '@modelcontextprotocol/server-filesystem', dir.trim()];
  }
  const j = await postAction(body, {quiet: true});
  if (j.ok) {
    await load();
    renderStoreItems();
  } else { btn.disabled = false; btn.innerHTML = ic('refresh') + ' R&eacute;essayer'; toast(j.error || 'erreur', 'err', 4000); }
}

/* ===== Barre latérale ajustable (largeur mémorisée via data-w, 0 style inline) & repliable (Ctrl+B) ===== */
const snapExplW = w => Math.round(Math.min(520, Math.max(180, w)) / 10) * 10;
function toggleExplorer() {
  const expl = document.getElementById('explorer');
  const btn = document.getElementById('tbExplToggle');
  if (!expl) return;
  const collapsed = expl.classList.toggle('collapsed');
  if (btn) btn.setAttribute('aria-expanded', collapsed ? 'false' : 'true');
  try { localStorage.setItem('mcc.explorer.collapsed', collapsed ? '1' : '0'); } catch (e) {}
}
(function () {
  const grip = document.getElementById('grip'), expl = document.getElementById('explorer');
  if (!grip || !expl) return;
  try {
    const saved = +localStorage.getItem('mcc.explorer.w');
    if (saved) expl.dataset.w = String(snapExplW(saved));
    if (localStorage.getItem('mcc.explorer.collapsed') === '1') {
      expl.classList.add('collapsed');
      const btn = document.getElementById('tbExplToggle');
      if (btn) btn.setAttribute('aria-expanded', 'false');
    }
  } catch (e) {}
  grip.addEventListener('mousedown', e => {
    e.preventDefault();
    grip.classList.add('drag');
    const x0 = e.clientX, w0 = expl.getBoundingClientRect().width;
    const mv = ev => { expl.dataset.w = String(snapExplW(w0 + ev.clientX - x0)); };
    const up = () => {
      try { localStorage.setItem('mcc.explorer.w', String(snapExplW(expl.getBoundingClientRect().width))); } catch (e) {}
      grip.classList.remove('drag');
      document.removeEventListener('mousemove', mv);
      document.removeEventListener('mouseup', up);
    };
    document.addEventListener('mousemove', mv);
    document.addEventListener('mouseup', up);
  });
})();

/* ===== Contrôles de fenêtre (desktop Tauri uniquement) ===== */
function tauriInvoke() {
  try {
    const I = window.__TAURI_INTERNALS__;
    if (I && typeof I.invoke === 'function') {
      return cmd => I.invoke(cmd, {});
    }
    const W = window.__TAURI__ && window.__TAURI__.window;
    if (!W) return null;
    const tw = W.getCurrentWindow ? W.getCurrentWindow() : (W.getCurrent ? W.getCurrent() : null);
    if (!tw) return null;
    return cmd => {
      if (cmd === 'plugin:window|start_dragging') return tw.startDragging();
      if (cmd === 'plugin:window|minimize') return tw.minimize();
      if (cmd === 'plugin:window|toggle_maximize') return tw.toggleMaximize();
      if (cmd === 'plugin:window|close') return tw.close();
    };
  } catch (e) { return null; }
}
/* Boutons de fenêtre visibles seulement si l'API fenêtre existe (desktop) :
   hors Tauri ils resteraient des boutons morts. Le drag + double-clic sur
   #titlebar est géré nativement par le script drag.js de Tauri 2 via
   data-tauri-drag-region="deep" (un seul appel IPC, zéro course). */
(function () {
  const inv = tauriInvoke();
  if (!inv) return;
  const box = document.getElementById('winCtrls');
  if (box) box.classList.add('on');
  // Filet de sécurité : écouteur sur WINDOW (phase bulle, donc APRÈS celui de
  // drag.js posé sur document). Si drag.js a pris le clic, il a appelé
  // stopImmediatePropagation → on ne reçoit rien (pas de double appel). S'il
  // ne l'a pas pris (script non injecté sur l'URL loopback, régression
  // Tauri…), on lance nous-mêmes le déplacement.
  window.addEventListener('mousedown', ev => {
    if (ev.button !== 0 || ev.defaultPrevented) return;
    const t = ev.target;
    if (!t || !t.closest || !t.closest('#titlebar')) return;
    if (t.closest('button,a,input,select,textarea,label,[role="menu"],[role="menuitem"],[tabindex]:not([tabindex="-1"]),#winCtrls')) return;
    ev.preventDefault();
    const cmd = ev.detail === 2 ? 'plugin:window|toggle_maximize' : 'plugin:window|start_dragging';
    try { Promise.resolve(inv(cmd)).catch(err => console.warn('[mcc] ' + cmd + ' refusé :', err)); }
    catch (e) { console.warn('[mcc] ' + cmd + ' a levé :', e); }
  });
})();
function winAction(op) {
  const inv = tauriInvoke();
  if (!inv) return;
  const cmds = {
    min: 'plugin:window|minimize',
    max: 'plugin:window|toggle_maximize',
    close: 'plugin:window|close',
  };
  const cmd = cmds[op];
  if (!cmd) return;
  try { Promise.resolve(inv(cmd)).catch(() => {}); } catch (e) { /* API refusée : aucun effet */ }
}

/* Journal replié PAR DÉFAUT (interface épurée) : l'opérateur l'ouvre d'un clic
   sur sa barre et son choix est retenu (localStorage). */
function toggleLog() {
  const lp = document.getElementById('logpanel');
  lp.classList.toggle('min');
  try { localStorage.setItem('mcc.logmin', String(lp.classList.contains('min'))); } catch (e) {}
}
(function () {
  let v = null;
  try { v = localStorage.getItem('mcc.logmin'); } catch (e) {}
  document.getElementById('logpanel').classList.toggle('min', v === null ? true : v === 'true');
})();
/* Helpers pour les onglets Identité & Sessions, Guide d'appairage et Verdicts
   (toutes les interpolations passent par esc() / ic()). */
function copyDataText(el) {
  const txt = el && el.dataset ? el.dataset.text : '';
  if (txt) copyText(txt);
}
function pairGuideHtml(uiUrl, sampleKeyId) {
  const url = uiUrl || (window.location && window.location.origin) || 'http://127.0.0.1:5090';
  const kid = sampleKeyId || '<keyid>';
  const cmdPair = 'mcp-control auth pair ' + url;
  const jsonBridge = JSON.stringify({
    mcpServers: {
      'mcp-control': {
        command: 'mcp-control',
        args: ['bridge', url, '--key', kid]
      }
    }
  }, null, 2);
  const blk = (title, code) =>
    `<div class="conf-block"><div class="conf-head">${esc(title)}<button data-act="copy-conf">copier</button></div><pre>${String(code || '').split('\n').map(esc).join('\n')}</pre></div>`;
  return `<div class="acc-sub">Connecter un client / assistant IA</div>
    <div class="acc-card">
      <div class="acc-who"><b>1. Appairer la machine cliente</b> &mdash; ex&eacute;cutez cette commande puis validez le code &agrave; 6 chiffres ci-dessus :</div>
      ${blk('Terminal — Demande d’appairage Ed25519', cmdPair)}
      <div class="acc-who"><b>2. Configurer Claude Desktop / Claude Code / Cursor</b> &mdash; le pont local signe chaque requ&ecirc;te RFC 9421 :</div>
      ${blk('JSON mcpServers — Pont signé mcp-control bridge', jsonBridge)}
    </div>`;
}
function renderAccIdentity() {
  const box = document.getElementById('accIdentity');
  if (!box || !accessState) return;
  const id = accessState.server_identity || {};
  const tls = accessState.tls || {};
  const sess = accessState.sessions || {active: (state && state.sessions_active) || 1, max: 8};
  const storageBadge = id.storage === 'keyring'
    ? `<span class="acc-tag ok">${ic('check')} Trousseau syst&egrave;me (keyring)</span>`
    : (id.storage === 'file_fallback'
      ? `<span class="acc-tag warn">${ic('warn')} Repli fichier local (file_fallback)</span>`
      : `<span class="acc-tag bad">${ic('x')} Indisponible (${esc(id.storage || 'unavailable')})</span>`);
  const storageWarn = id.storage === 'file_fallback'
    ? `<div class="acc-who tone-warn">${ic('warn')} La cl&eacute; priv&eacute;e du serveur est stock&eacute;e en fichier local (trousseau syst&egrave;me indisponible ou d&eacute;sactiv&eacute;).</div>`
    : (id.storage && id.storage !== 'keyring'
      ? `<div class="acc-who tone-bad">${ic('warn')} Identit&eacute; cryptographique du serveur non disponible.</div>`
      : '');
  const tlsBadge = tls.bind_refused
    ? `<span class="acc-tag bad">${ic('x')} Bind LAN refus&eacute; (sans TLS)</span>`
    : (tls.active
      ? `<span class="acc-tag ok">${ic('lock')} TLS 1.3 actif</span>`
      : (tls.compiled
        ? `<span class="acc-tag">${ic('shield')} Loopback HTTP (TLS 1.3 compil&eacute;)</span>`
        : `<span class="acc-tag warn">Non compil&eacute; (loopback seul)</span>`));
  const verifyCmd = 'mcp-control auth verify-log';
  box.innerHTML = `
    <div class="acc-card">
      <div class="acc-h">
        <span class="acc-tool">Identit&eacute; du serveur (${esc(id.keyid || 'mcc-server')})</span>
        ${storageBadge}
      </div>
      ${storageWarn}
      <div class="acc-kv"><span class="acc-k">keyid</span><span class="acc-v">${esc(id.keyid || 'mcc-server')}</span></div>
      <div class="acc-kv"><span class="acc-k">empreinte courte</span><span class="acc-v">${esc(id.fingerprint_court || '—')}</span></div>
      <div class="acc-kv"><span class="acc-k">empreinte compl&egrave;te</span><span class="acc-v">${esc(id.fingerprint || '—')}</span></div>
      ${id.fingerprint ? `<div class="acc-acts"><button data-act="copy-text" data-text="${esc(id.fingerprint)}">${ic('copy')} Copier l'empreinte serveur</button></div>` : ''}
    </div>
    <div class="acc-card">
      <div class="acc-h">
        <span class="acc-tool">R&eacute;seau &amp; TLS 1.3</span>
        ${tlsBadge}
      </div>
      <div class="acc-kv"><span class="acc-k">URL d'acc&egrave;s</span><span class="acc-v">${esc(accessState.ui_url || '—')}</span></div>
      <div class="acc-kv"><span class="acc-k">certificat TLS</span><span class="acc-v">${esc(tls.fingerprint || 'aucun (loopback)')}</span></div>
      ${tls.notice ? `<div class="acc-who">${esc(tls.notice)}</div>` : ''}
      ${tls.fingerprint ? `<div class="acc-acts"><button data-act="copy-text" data-text="${esc(tls.fingerprint)}">${ic('copy')} Copier l'empreinte TLS</button></div>` : ''}
    </div>
    <div class="acc-card">
      <div class="acc-h">
        <span class="acc-tool">Sessions locales</span>
        <span class="acc-tag ok">${esc(sess.active ?? 1)} / ${esc(sess.max ?? 8)} sessions actives</span>
      </div>
      <div class="acc-who">Chaque onglet local authentifi&eacute; d&eacute;tient un cookie HttpOnly <code>mcc_session</code> coupl&eacute; &agrave; un secret en m&eacute;moire (<code>X-MCC-Session</code>).</div>
      <div class="acc-acts">
        <button class="danger" data-act="close-other-sessions">${ic('lock')} Fermer les autres sessions</button>
      </div>
    </div>
    <div class="acc-card">
      <div class="acc-h">
        <span class="acc-tool">Journal d'audit cha&icirc;n&eacute; (SHA-256)</span>
        <span class="acc-tag">${ic('shield')} audit.jsonl</span>
      </div>
      <div class="acc-who">Chaque d&eacute;cision d'authentification et d'autorisation sur <code>/mcp</code> est scell&eacute;e par le hachage SHA-256 de l'entr&eacute;e pr&eacute;c&eacute;dente. V&eacute;rifiez l'int&eacute;grit&eacute; de la cha&icirc;ne avec :</div>
      <div class="conf-block"><div class="conf-head">V&eacute;rification d'int&eacute;grit&eacute; du journal<button data-act="copy-conf">copier</button></div><pre>${esc(verifyCmd)}</pre></div>
    </div>`;
}
async function closeOtherSessions() {
  try {
    const r = await fetch('/api/session/close-others', {
      method: 'POST',
      headers: {'Content-Type': 'application/json'},
      body: '{}'
    });
    const j = await r.json().catch(() => null);
    if (r.ok && j && j.ok) {
      const n = j.closed || 0;
      toast(n + ' autre' + (n > 1 ? 's' : '') + ' session' + (n > 1 ? 's' : '') + ' ferm\u00e9e' + (n > 1 ? 's' : ''), 'ok');
      await refreshAccess();
      await load();
    } else {
      accessMsg('\u2717 ' + ((j && j.error) || 'impossible de fermer les autres sessions'));
    }
  } catch (e) {
    accessMsg('\u2717 erreur r\u00e9seau lors de la fermeture des sessions');
  }
}
function verdictTone(v) {
  const s = String(v || '').toLowerCase();
  if (s.includes('allow') || s.includes('autoris') || s === 'ok' || s.includes('approv')) return 'ok';
  if (s.includes('expir')) return 'warn';
  return 'bad';
}
function verdictIcon(v) {
  const s = String(v || '').toLowerCase();
  if (s.includes('allow') || s.includes('autoris') || s === 'ok' || s.includes('approv')) return 'check';
  if (s.includes('expir')) return 'warn';
  if (s.includes('revo')) return 'lock';
  return 'x';
}
/* Ticker local 1000 ms pour les comptes à rebours de #accessDlg : met uniquement
   à jour .textContent des éléments [data-ttl-until] sans requête réseau ni innerHTML. */
function tickAccessTtl() {
  const dlg = document.getElementById('accessDlg');
  if (!dlg || !dlg.open) return;
  const now = Date.now();
  dlg.querySelectorAll('[data-ttl-until]').forEach(el => {
    const until = +el.dataset.ttlUntil || 0;
    const rem = Math.max(0, Math.ceil((until - now) / 1000));
    const txt = el.dataset.ttlMode === 'grant'
      ? (rem + ' s restantes')
      : ('expire dans ' + rem + ' s');
    setText(el, txt);
  });
}

/* ===== Écran « Accès » (lot M3, docs/auth.md §5) ========================
   Cinq volets : confirmations en attente (ce qui va être lancé), appairage
   (demandes + guide), clients appairés (droits, inactivité > 180 j, rotation,
   révocation), identité & sessions, 100 dernières décisions. TOUT texte venu
   d'un client passe par esc() : un libellé hostile ne doit jamais devenir du
   HTML (test « libellé hostile » côté Rust). Les ordres partent par postAction
   (session locale) — jamais un fetch direct vers /api/action. */
let accessState = null;
let accessTab = 'confirmations';
let accessTimer = null;
let clientFilterMode = 'all';
let decisionFilterQuery = '';
const pairDroits = {}; // pair_id -> droits choisis localement (défaut : lecture)

function accessMsg(txt) {
  const el = document.getElementById('accessMsg');
  if (el) el.textContent = txt || '';
}
function accessBadge() {
  const a = (state && state.access) || {};
  const n = (a.pending || 0) + (a.pairs || 0);
  const b = document.getElementById('accessBanner');
  if (b) {
    setHidden(b, !n);
    if (n) {
      const parts = [];
      if (a.pending) parts.push(a.pending + ' confirmation' + (a.pending > 1 ? 's' : '') + ' avant ex\u00e9cution');
      if (a.pairs) parts.push(a.pairs + ' demande' + (a.pairs > 1 ? 's' : '') + ' d\u2019appairage');
      const html = ic('shield') + '<span>' + parts.join(' et ') + ' en attente &mdash; cliquer pour d\u00e9cider</span>';
      if (b.innerHTML !== html) b.innerHTML = html;
    }
  }
  const badge = document.getElementById('tbAccessN');
  if (badge) { setHidden(badge, !n); setText(badge, n); }
  const cp = document.getElementById('accCountPending');
  if (cp) { setHidden(cp, !(a.pending || 0)); setText(cp, a.pending || 0); }
  const cpa = document.getElementById('accCountPairs');
  if (cpa) { setHidden(cpa, !(a.pairs || 0)); setText(cpa, a.pairs || 0); }
}
function openAccessDlg(pending) {
  const d = document.getElementById('accessDlg');
  if (!d) return;
  rememberFocus();
  let tab = 'confirmations';
  if (!pending) { try { tab = localStorage.getItem('mcc.acctab') || tab; } catch (e) {} }
  setAccessTab(tab);
  accessMsg('');
  d.showModal();
  refreshAccess();
  if (accessTimer) clearInterval(accessTimer);
  accessTimer = setInterval(tickAccessTtl, 1000);
}
function closeAccessDlg() {
  if (accessTimer) { clearInterval(accessTimer); accessTimer = null; }
  const d = document.getElementById('accessDlg');
  if (d && d.open) d.close();
  restoreFocus();
}
function setAccessTab(name) {
  accessTab = name;
  document.querySelectorAll('#accessDlg .set-nav-btn').forEach(b => {
    const sel = b.dataset.atab === name;
    b.classList.toggle('sel', sel);
    b.setAttribute('aria-selected', sel ? 'true' : 'false');
    b.setAttribute('tabindex', sel ? '0' : '-1');
  });
  document.querySelectorAll('#accessDlg .set-pane').forEach(p => p.classList.toggle('sel', p.dataset.atab === name));
  try { localStorage.setItem('mcc.acctab', name); } catch (e) {}
}
async function refreshAccess() {
  const j = await postAction({action: 'access_list'}, {quiet: true});
  if (!j.ok) { accessMsg('\u2717 ' + (j.error || 'lecture impossible')); return; }
  accessState = j;
  accessMsg('');
  renderAccess();
}
function renderAccess() {
  if (!accessState) return;
  const sig = JSON.stringify([accessState, clientFilterMode, decisionFilterQuery, pairDroits]);
  if (sig === _sigAccess) return;
  _sigAccess = sig;
  renderAccPending();
  renderAccClients();
  renderAccPairs();
  renderAccIdentity();
  renderAccDecisions();
  accessBadge();
}
/* Effet exact d'un appel : tout est échappé, rien n'est interprété. */
function effectHtml(e) {
  if (!e || typeof e !== 'object') return '';
  const out = [];
  const add = (k, v) => {
    const t = v === undefined || v === null ? '' : String(v);
    if (t) out.push('<div class="acc-kv"><span class="acc-k">' + esc(k) + '</span><span class="acc-v">' + esc(t) + '</span></div>');
  };
  if (e.kind === 'lancement') {
    add('programme', e.program);
    if (Array.isArray(e.args) && e.args.length) add('arguments', e.args.join(' '));
    add('dossier de travail', e.workdir);
    if (Array.isArray(e.env) && e.env.length) add('variables', e.env.join(', ') + ' (valeurs masquées)');
  } else if (e.kind === 'outil') {
    add('serveur', e.target);
    add('outil', e.tool);
    if (e.arguments !== undefined) add('arguments', JSON.stringify(e.arguments));
  } else if (e.kind === 'ssh') {
    add('cible', e.target);
  } else if (e.kind === 'import') {
    (e.servers || []).forEach((s, i) => add('serveur ' + (i + 1),
      [s.label, s.program].filter(Boolean).join(' \u2014 ') +
      (Array.isArray(s.args) && s.args.length ? ' ' + s.args.join(' ') : '')));
  } else {
    add('cible', e.target || e.label);
  }
  if (e.note) add('note', e.note);
  return out.join('');
}
function renderAccPending() {
  const box = document.getElementById('accPending');
  const grantsBox = document.getElementById('accGrants');
  if (!box || !grantsBox) return;
  const nowMs = Date.now();
  const pend = accessState.pending || [];
  const ttl = accessState.request_ttl_secs || 120;
  let html = pend.length ? '' :
    '<div class="empty-sec">aucune confirmation en attente \u2014 un appel d\u2019outil ' +
    '<code>execution</code> appara\u00eetra ici et rien ne sera lanc\u00e9 sans votre accord</div>';
  html += pend.map(p => `<div class="acc-card warn">
    <div class="acc-h">
      <span class="acc-tool">${esc(p.tool)}</span>
      <span class="acc-ttl" data-ttl-until="${esc(nowMs + Number(p.expires_in || 0) * 1000)}" data-ttl-mode="pending" title="une demande non d\u00e9cid\u00e9e expire au bout de ${esc(ttl)} s">expire dans ${esc(p.expires_in)} s</span>
    </div>
    <div class="acc-who">${esc(p.label || p.keyid)} <span class="acc-fp" title="${esc(p.fingerprint)}">${esc(String(p.fingerprint || '').slice(0, 24))}</span> depuis <b>${esc(p.address)}</b></div>
    <div class="acc-what">${esc((p.effect || {}).resume || '')}</div>
    ${effectHtml(p.effect)}
    <div class="acc-acts">
      <button class="primary" data-act="confirm-approve" data-id="${esc(p.id)}" data-mode="once">Accepter une fois</button>
      <button class="primary" data-act="confirm-approve" data-id="${esc(p.id)}" data-mode="window">Accepter ${esc(Math.round((accessState.window_secs || 600) / 60))} minutes</button>
      <button class="danger" data-act="confirm-deny" data-id="${esc(p.id)}">Refuser</button>
    </div>
  </div>`).join('');
  box.innerHTML = html;

  const grants = accessState.grants || [];
  let gh = '<div class="acc-sub">Permis en cours</div>';
  gh += grants.length ? '' : '<div class="empty-sec">aucun permis actif</div>';
  gh += grants.map(g => `<div class="acc-card ok">
    <div class="acc-h">
      <span class="acc-tool">${esc(g.tool)}</span>
      <span class="acc-tag">${esc(g.mode === 'once' ? 'une fois' : 'fen\u00eatre')}</span>
      <span class="acc-ttl" data-ttl-until="${esc(nowMs + Number(g.expires_in || 0) * 1000)}" data-ttl-mode="grant">${esc(g.expires_in)} s restantes</span>
    </div>
    <div class="acc-who">${esc(g.label || g.keyid)}</div>
    <div class="acc-acts"><button class="danger" data-act="grant-revoke" data-id="${esc(g.id)}">R\u00e9voquer</button></div>
  </div>`).join('');
  grantsBox.innerHTML = gh;
}
function setClientFilter(mode) {
  clientFilterMode = mode || 'all';
  renderAccClients();
}
function renderAccClients() {
  const box = document.getElementById('accClients');
  if (!box) return;
  const allClients = accessState.clients || [];
  const clients = allClients.filter(c => {
    if (clientFilterMode === 'active') return c.status !== 'revoked';
    if (clientFilterMode === 'revoked') return c.status === 'revoked';
    return true;
  });
  let html = '<div class="acc-toolbar" role="group" aria-label="Filtrer les clients">' +
    '<button class="exp-pill' + (clientFilterMode === 'all' ? ' sel' : '') + '" data-act="client-filter" data-mode="all">Tous (' + esc(allClients.length) + ')</button>' +
    '<button class="exp-pill' + (clientFilterMode === 'active' ? ' sel' : '') + '" data-act="client-filter" data-mode="active">Actifs (' + esc(allClients.filter(c => c.status !== 'revoked').length) + ')</button>' +
    '<button class="exp-pill' + (clientFilterMode === 'revoked' ? ' sel' : '') + '" data-act="client-filter" data-mode="revoked">R\u00e9voqu\u00e9s (' + esc(allClients.filter(c => c.status === 'revoked').length) + ')</button>' +
    '</div>';
  html += clients.length ? '' : '<div class="empty-sec">aucun client appair\u00e9 dans cette vue</div>';
  html += clients.map(c => {
    const droits = c.rights || [];
    const boutons = ['lecture', 'pilotage', 'execution'].map(r => {
      const classeOn = droits.includes(r) ? ' on' : '';
      return `<button class="acc-right${classeOn}" role="switch" aria-checked="${esc(droits.includes(r) ? 'true' : 'false')}" data-act="client-right" data-id="${esc(c.keyid)}" data-right="${esc(r)}"
        title="${esc(droits.includes(r) ? 'Retirer' : 'Accorder')} le droit ${esc(r)}">${esc(r)}</button>`;
    }).join('');
    const revoque = c.status === 'revoked';
    const classeOff = revoque ? ' off' : '';
    const badgeRevoque = (revoque ? '<span class="acc-tag bad">' + esc('révoqué') + '</span>' : '<span class="acc-tag ok">' + esc('actif') + '</span>')
      + (c.stale_180d ? ' <span class="acc-tag warn" title="Aucune utilisation depuis plus de 180 jours">Inactif &gt; 180 j</span>' : '');
    const btnRevoque = revoque ? ''
      : '<button data-act="client-rotate" data-id="' + esc(c.keyid) + '">Rotation de cl\u00e9</button>'
        + '<button class="danger" data-act="client-revoke" data-id="' + esc(c.keyid) + '">R\u00e9voquer</button>';
    return `<div class="acc-card${classeOff}">
      <div class="acc-h">
        <span class="acc-tool">${esc(c.label || c.keyid)}</span>
        <span class="acc-tag">${esc(c.keyid)}</span>
        ${badgeRevoque}
      </div>
      <div class="acc-who"><span class="acc-fp" title="${esc(c.fingerprint)}">${esc(c.fingerprint_court || c.fingerprint)}</span>
        \u00b7 appair\u00e9 depuis ${esc(c.paired_from || '?')}
        \u00b7 cr\u00e9\u00e9 le ${esc(c.created_at || '?')}
        \u00b7 derni\u00e8re utilisation : ${esc(c.last_used_at || 'jamais')}</div>
      <div class="acc-acts">${boutons}${btnRevoque}
      </div>
    </div>`;
  }).join('');
  box.innerHTML = html;
}

function renderAccPairs() {
  const box = document.getElementById('accPairs');
  if (!box) return;
  const nowMs = Date.now();
  const pairs = accessState.pairs || [];
  let html = pairs.length ? '' :
    '<div class="empty-sec">aucune demande d\u2019appairage en attente \u2014 un client qui lance <code>mcp-control auth pair</code> appara\u00eetra ici</div>';
  html += pairs.map(p => {
    if (!pairDroits[p.pair_id]) pairDroits[p.pair_id] = ['lecture'];
    const choisis = pairDroits[p.pair_id];
    const cases = ['lecture', 'pilotage', 'execution'].map(r =>
      `<label class="acc-chk"><input type="checkbox" data-pair="${esc(p.pair_id)}" data-right="${esc(r)}"
        ${esc(choisis.includes(r) ? 'checked' : '')} data-act="pair-right">${esc(r)}</label>`).join('');
    return `<div class="acc-card">
      <div class="acc-h">
        <span class="acc-code" title="le client affiche le m\u00eame code : s\u2019ils diff\u00e8rent, refusez">${esc(p.code)}</span>
        <span class="acc-ttl" data-ttl-until="${esc(nowMs + Number(p.expires_in || 0) * 1000)}" data-ttl-mode="pending">expire dans ${esc(p.expires_in)} s</span>
      </div>
      <div class="acc-who">${esc(p.label || p.keyid)} <span class="acc-fp" title="${esc(p.fingerprint)}">${esc(p.fingerprint_court || '')}</span>
        \u00b7 ${esc(p.keyid)} \u00b7 depuis ${esc(p.paired_from || '?')}</div>
      <div class="acc-acts">${cases}
        <button class="primary" data-act="pair-approve" data-id="${esc(p.pair_id)}">Accepter</button>
        <button class="danger" data-act="pair-reject" data-id="${esc(p.pair_id)}">Refuser</button>
      </div>
    </div>`;
  }).join('');
  const firstClient = ((accessState.clients || []).find(c => c.status !== 'revoked') || {}).keyid || '<keyid>';
  html += pairGuideHtml(accessState.ui_url, firstClient);
  box.innerHTML = html;
}
function setDecisionFilter(el) {
  decisionFilterQuery = (el && el.value ? el.value : '').trim().toLowerCase();
  renderAccDecisionsList();
}
function renderAccDecisions() {
  const box = document.getElementById('accDecisions');
  if (!box) return;
  let listBox = document.getElementById('accDecisionsRows');
  if (!listBox) {
    box.innerHTML = '<div class="acc-toolbar">' +
      '<label for="accDecisionFilter" class="sr-only">Filtrer le journal d\u2019audit</label>' +
      '<input id="accDecisionFilter" placeholder="Filtrer par outil, client, verdict ou motif\u2026" data-act="decision-filter" value="' + esc(decisionFilterQuery) + '">' +
      '</div><div id="accDecisionsRows"></div>';
  }
  renderAccDecisionsList();
}
function renderAccDecisionsList() {
  const rowsBox = document.getElementById('accDecisionsRows');
  if (!rowsBox || !accessState) return;
  const all = accessState.decisions || [];
  const q = decisionFilterQuery;
  const d = q ? all.filter(x => {
    const hay = ((x.verdict || '') + ' ' + (x.tool || '') + ' ' + (x.keyid || '') + ' ' + (x.reason || '')).toLowerCase();
    return hay.includes(q);
  }) : all;
  const heure = ts => {
    try { return new Date(Number(ts) * 1000).toLocaleTimeString(); } catch (e) { return '?'; }
  };
  let html = d.length ? '' : '<div class="empty-sec">aucune d\u00e9cision enregistr\u00e9e pour ce filtre</div>';
  html += d.map(x => `<div class="acc-line">
    <span class="acc-when">${esc(heure(x.ts))}</span>
    <span class="acc-verdict"><span class="acc-tag ${esc(verdictTone(x.verdict))}">${ic(verdictIcon(x.verdict))} ${esc(x.verdict)}</span></span>
    <span class="acc-tool">${esc(x.tool)}</span>
    <span class="acc-who-sm">${esc(x.keyid)}</span>
    <span class="acc-reason">${esc(x.reason === '-' ? '' : (x.reason || ''))}</span>
  </div>`).join('');
  rowsBox.innerHTML = html;
}
async function confirmApprove(id, mode) {
  const j = await postAction({action: 'confirm_approve', request_id: id, mode}, {quiet: true});
  if (!j.ok) { accessMsg('\u2717 ' + (j.error || 'refus')); return; }
  toast(mode === 'once' ? 'confirm\u00e9 : une seule ex\u00e9cution' : 'confirm\u00e9 pour la dur\u00e9e du permis', 'ok');
  await refreshAccess();
  await load();
}
async function confirmDeny(id) {
  const j = await postAction({action: 'confirm_deny', request_id: id}, {quiet: true});
  if (!j.ok) { accessMsg('\u2717 ' + (j.error || 'refus')); return; }
  toast('demande refus\u00e9e : rien ne sera ex\u00e9cut\u00e9', 'ok');
  await refreshAccess();
  await load();
}
async function grantRevoke(id) {
  const j = await postAction({action: 'grant_revoke', request_id: id}, {quiet: true});
  if (!j.ok) { accessMsg('\u2717 ' + (j.error || 'refus')); return; }
  await refreshAccess();
}
async function clientToggleRight(keyid, right) {
  const c = (accessState.clients || []).find(x => x.keyid === keyid);
  if (!c) return;
  const actuels = c.rights || [];
  const droits = actuels.includes(right) ? actuels.filter(r => r !== right) : actuels.concat([right]);
  const j = await postAction({action: 'client_set_scopes', keyid, rights: droits}, {quiet: true});
  if (!j.ok) { accessMsg('\u2717 ' + (j.error || 'refus')); return; }
  await refreshAccess();
}
async function clientRotateModal(keyid) {
  const res = await askConfirm({
    title: 'Rotation de cl\u00e9 Ed25519',
    msg: 'Remplacer la cl\u00e9 publique du client ' + keyid + ' en conservant ses droits :',
    inputs: [
      {id: 'new_keyid', label: 'Nouveau keyid (mcc-<nom>-<16hex>)', placeholder: 'mcc-cli-0123456789abcdef'},
      {id: 'public_key', label: 'Nouvelle cl\u00e9 publique Ed25519 (64 hex)', placeholder: '64 caract\u00e8res hexad\u00e9cimaux minuscules'}
    ],
    okLabel: 'Effectuer la rotation',
  });
  if (!res || typeof res !== 'object') return;
  if (!res.new_keyid || !res.public_key) {
    accessMsg('\u2717 nouveau keyid et cl\u00e9 publique requis');
    return;
  }
  const j = await postAction({action: 'client_rotate', keyid, new_keyid: res.new_keyid, public_key: res.public_key}, {quiet: true});
  if (!j.ok) { accessMsg('\u2717 ' + (j.error || 'rotation refus\u00e9e')); return; }
  toast('rotation effectu\u00e9e : ' + (j.keyid || res.new_keyid), 'ok');
  await refreshAccess();
}
async function clientRevokeConfirm(keyid) {
  const ok = await askConfirm({
    title: 'R\u00e9voquer ce client',
    msg: 'R\u00e9voquer ' + keyid + ' ? Ses requ\u00eates seront refus\u00e9es d\u00e8s la suivante.',
    okLabel: 'R\u00e9voquer',
    danger: true,
  });
  if (!ok) return;
  const j = await postAction({action: 'client_revoke', keyid}, {quiet: true});
  if (!j.ok) { accessMsg('\u2717 ' + (j.error || 'refus')); return; }
  await refreshAccess();
}
function pairRightChange(el) {
  const id = el.dataset.pair;
  const droit = el.dataset.right;
  const liste = pairDroits[id] || [];
  if (el.checked) { if (!liste.includes(droit)) liste.push(droit); } else {
    const i = liste.indexOf(droit);
    if (i >= 0) liste.splice(i, 1);
  }
  pairDroits[id] = liste;
}
async function pairApprove(pairId) {
  const droits = pairDroits[pairId] || ['lecture'];
  const j = await postAction({action: 'pair_approve', pair_id: pairId, rights: droits}, {quiet: true});
  if (!j.ok) { accessMsg('\u2717 ' + (j.error || 'refus')); return; }
  delete pairDroits[pairId];
  toast('client appair\u00e9', 'ok');
  await refreshAccess();
  await load();
}
async function pairReject(pairId) {
  const j = await postAction({action: 'pair_reject', pair_id: pairId}, {quiet: true});
  if (!j.ok) { accessMsg('\u2717 ' + (j.error || 'refus')); return; }
  delete pairDroits[pairId];
  await refreshAccess();
}
/* Esc ferme l'écran Accès (aucune saisie en cours : rien à jeter). */
(function () {
  const d = document.getElementById('accessDlg');
  if (!d) return;
  d.addEventListener('cancel', () => { if (accessTimer) { clearInterval(accessTimer); accessTimer = null; } restoreFocus(); });
})();

/* ===== Polling armé sur les réglages =====
   Rythme normal : settings.refresh_secs (min 5 s), ré-armé à chaque état
   reçu. Panne moteur (lot C #2) : après le PREMIER échec de load(), réessai
   RAPIDE à 2 s (POLL_RETRY_MS) au lieu du prochain cycle complet — bandeau
   toujours au 2ᵉ échec, mais ~refresh_secs+2 s au lieu de ~2×refresh_secs ;
   pendant la panne, chaque échec réarme un réessai de 2 s (fourchette mission
   2 à 5 s) ; dès que le moteur répond, render() → armPolling() revient au
   rythme normal. Un seul timer à la fois : scheduleNext remplace le
   setTimeout en attente (fenêtre masquée : load() sort tôt, la chaîne reprend
   au retour par visibilitychange). */
let pollTimer = null;
const POLL_RETRY_MS = 2000; // panne : réessai rapide (lot C #2)
function scheduleNext(ms) {
  if (pollTimer) clearTimeout(pollTimer);
  pollTimer = setTimeout(load, ms);
}
function armPolling() {
  const secs = Math.max(5, (state && state.settings && state.settings.refresh_secs) || 5);
  scheduleNext(secs * 1000);
}
/* ===== Table action → fonction (lot S2) ==================================
   AUCUN attribut on* nulle part : chaque contrôle porte data-act (clé de
   cette table) et, au besoin, data-id / data-op / data-on / data-tab /
   data-mode / data-cat / data-key / data-tool / data-pair / data-right /
   data-i. Les valeurs data-* sont des CHAÎNES relues dans dataset — jamais
   évaluées, jamais concaténées dans du code : une valeur venue du moteur ne
   peut pas devenir du JavaScript. Tout texte affiché passe par esc() au
   moment où le HTML est écrit. */
/* ===== Menu « Découverte » (barre de titre : ports, réseau, machines) ===== */
function closeDiscoverMenu() {
  const menu = document.getElementById('discoverMenu');
  const btn = document.getElementById('tbDiscover');
  if (menu) menu.hidden = true;
  if (btn) btn.setAttribute('aria-expanded', 'false');
}
function toggleDiscoverMenu() {
  const menu = document.getElementById('discoverMenu');
  const btn = document.getElementById('tbDiscover');
  if (!menu || !btn) return;
  const open = menu.hidden;
  menu.hidden = !open;
  btn.setAttribute('aria-expanded', open ? 'true' : 'false');
  if (open) {
    const first = menu.querySelector('.tb-menu-item');
    if (first) first.focus();
  }
}
document.addEventListener('click', ev => {
  const menu = document.getElementById('discoverMenu');
  if (!menu || menu.hidden) return;
  const wrap = document.getElementById('tbDiscoverWrap');
  if (wrap && !wrap.contains(ev.target)) closeDiscoverMenu();
});
document.addEventListener('keydown', ev => {
  if (ev.key !== 'Escape') return;
  const menu = document.getElementById('discoverMenu');
  if (menu && !menu.hidden) {
    closeDiscoverMenu();
    const btn = document.getElementById('tbDiscover');
    if (btn) btn.focus();
  }
});

const ACT = {
  // -- barre de titre, bandeaux, journal
  'toggle-explorer': () => toggleExplorer(),
  'logout': () => logout(),
  'discover-toggle': () => toggleDiscoverMenu(),
  'scan': () => { closeDiscoverMenu(); doScan(); },
  'scan-net': () => { closeDiscoverMenu(); doScanNet(); },
  'machines': () => { closeDiscoverMenu(); openMachineDlg(); },
  'store': () => openStore(),
  'access-open': el => openAccessDlg(el.dataset.pending === '1'),
  'settings-open': () => openSettingsDlg(),
  'refresh-all': () => refreshAll(),
  'log-toggle': () => toggleLog(),
  'net-detail': () => showNetDetail(),
  'autostart': () => toggleAutostart(),
  'update-download': () => downloadUpdate(),
  // -- contrôles de fenêtre (desktop Tauri : sans effet dans un navigateur)
  'win-min': () => winAction('min'),
  'win-max': () => winAction('max'),
  'win-close': () => winAction('close'),
  // -- explorateur
  'sort': el => applySort(el.value),
  'srv-filter': () => renderServers(),
  'clear-filter': () => clearFilter(),
  'exp-state': el => setExpStateFilter(el.dataset.mode),
  'show-hidden': () => showHiddenServers(),
  'group-toggle': el => toggleGroup(el.dataset.key),
  'add-server': () => toggleAdd(),
  'select': el => select(el.dataset.id),
  'row-act': el => act(el.dataset.op, el.dataset.id),
  'set-auto': el => setAuto(el.dataset.id, el.dataset.on === '1'),
  'set-fav': el => setFavorite(el.dataset.id, el.dataset.on === '1'),
  'hide-server': el => hideServer(el.dataset.id),
  'remove-srv': el => removeSrv(el.dataset.id),
  'remove-seed': el => removeSeedForever(el.dataset.id),
  'ssh-forget': el => sshForget(el.dataset.id),
  'import-peer': el => importPeer(el.dataset.id),
  'follow-ann': el => followAnnouncement(el.dataset.name, el.dataset.port),
  // -- accueil et détail serveur
  'tile': el => tileFilter(el.dataset.mode),
  'tile-tools': () => tileSortTools(),
  'detail-tab': el => { detailTab = el.dataset.tab; renderDetail(); },
  'tool-search': el => setToolSearch(el),
  'tools-all-on': el => setAllTools(el.dataset.id, true),
  'tools-all-off': el => setAllTools(el.dataset.id, false),
  'tool-expand': el => toggleToolExpand(el),
  'tool-toggle': el => toggleTool(el.dataset.id, el.dataset.tool),
  'copy-tool-schema': el => copyToolSchema(el.dataset.tool),
  'copy-tool-rpc': el => copyToolRpc(el.dataset.tool),
  'tool-args-input': el => saveToolArgsDraft(el),
  'tool-args-reset': el => resetToolArgsDraft(el.dataset.id, el.dataset.tool),
  'tool-run': el => runToolTest(el.dataset.id, el.dataset.tool),
  'server-pair': el => serverPair(el.dataset.id),
  'copy-self': el => copyText(el.textContent),
  'copy-text': el => copyDataText(el),
  'copy-conf': el => copyConf(el),
  'set-start-cmd': el => setStartCmd(el.dataset.id),
  'ctx-item': el => ctxItem(el),
  // -- boutique
  'store-cat': el => { storeCat = el.dataset.cat; renderStoreItems(); },
  'store-search': () => renderStoreItems(),
  'store-sort': () => renderStoreItems(),
  'store-hide-inst': () => toggleHideInst(),
  'toggle-env-eye': el => toggleEnvEye(el),
  'install-store': el => installStore(el.dataset.id, el),
  // -- machines distantes (SSH)
  'ssh-discover': el => sshDiscover(el.dataset.id, el.dataset.user),
  'ssh-save': () => sshSave(),
  'peers-scan': () => { doPeers().then(() => setTimeout(renderMachines, 3000)); },
  // -- réglages
  'set-pane': el => setPane(el.dataset.pane),
  'theme': el => applyThemeAndSave(el.dataset.theme),
  'test-notify': () => testNotify(),
  'check-update': () => checkUpdateNow(),
  'copy-sha256': () => copySha256(),
  'release-page': () => openReleasePage(),
  'settings-close': () => closeSettings(),
  'save-settings': () => saveSettings(),
  'reset-state': () => resetState(document.getElementById('sResetWhat').value),
  'reset-runtime': () => resetRuntime(),
  // -- écran Accès
  'access-close': () => closeAccessDlg(),
  'access-tab': el => setAccessTab(el.dataset.atab),
  'access-refresh': () => refreshAccess(),
  'confirm-approve': el => confirmApprove(el.dataset.id, el.dataset.mode),
  'confirm-deny': el => confirmDeny(el.dataset.id),
  'grant-revoke': el => grantRevoke(el.dataset.id),
  'client-filter': el => setClientFilter(el.dataset.mode),
  'client-right': el => clientToggleRight(el.dataset.id, el.dataset.right),
  'client-rotate': el => clientRotateModal(el.dataset.id),
  'client-revoke': el => clientRevokeConfirm(el.dataset.id),
  'pair-approve': el => pairApprove(el.dataset.id),
  'pair-reject': el => pairReject(el.dataset.id),
  'pair-right': el => pairRightChange(el),
  'close-other-sessions': () => closeOtherSessions(),
  'decision-filter': el => setDecisionFilter(el),
  // -- formulaires et dialogs
  'add-submit': (_el, ev) => addServer(ev),
  'login-submit': (_el, ev) => loginSubmit(ev),
  'transport': () => transportChanged(),
  'import-mcp': () => importMcp(),
  'forget-code': () => forgetCode(),
  'confirm-dlg-ok': () => confirmDlgOk(),
  'confirm-dlg-cancel': () => confirmDlgCancel(),
  'close-dialog': el => { const d = el.closest('dialog'); if (d) { d.close(); restoreFocus(); } },
};

/* Entrée du menu contextuel : l'index vient de data-i, la fonction du tableau
   d'items du menu courant — aucune valeur reçue n'est exécutée. */
function ctxItem(el) {
  const it = ctxItems[+el.dataset.i];
  if (!it || it.dis) return;
  ctxClose();
  it.fn();
}

/* Aiguillage unique : le contrôle le plus proche porteur de data-act gagne
   (un bouton dans une ligne n'arme pas l'action de la ligne), puis la table
   décide. Une clé inconnue ne fait rien — le test Rust interdit les clés
   orphelines dans les deux sens.
   Filtre de type : un <form data-act> n'agit qu'au SUBMIT (sinon un clic dans
   un champ déclencherait l'envoi) ; un <select>/<input data-act> n'agit qu'au
   change/input (sinon son clic vaudrait choix). */
const CHAMPS = ['INPUT', 'SELECT', 'TEXTAREA'];
const isNonTextChamp = el => el.tagName === 'SELECT'
  || (el.tagName === 'INPUT' && (el.type === 'checkbox' || el.type === 'radio'));
function runAction(ev) {
  const t = ev.target;
  const el = t && t.closest ? t.closest('[data-act]') : null;
  if (!el) return;
  const tag = el.tagName;
  if (ev.type === 'click' && (tag === 'FORM' || CHAMPS.includes(tag))) return;
  if ((ev.type === 'change' || ev.type === 'input') && !CHAMPS.includes(tag)) return;
  if (ev.type === 'input' && isNonTextChamp(el)) return;
  const fn = ACT[el.dataset.act];
  if (!fn) return;
  fn(el, ev);
}

loadFrameworks();
loadStoreLogos();
updateLogoutBtn();
load();
armPolling();
// retour de la fenêtre (tray → visible) : reprise immédiate du rafraîchissement
document.addEventListener('visibilitychange', () => { if (!document.hidden) load(); });
